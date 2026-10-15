//! `yourfs` — FUSE mount of a whitelisted, read-only view over an NFS
//! export. Rust rewrite of the C `yourfs.c` main().

use std::ffi::CString;

use yourfs::{cli, db, fs, key, libnfs, nfscli, paths};

fn parse_i64(s: &str) -> i64 {
    // C atoi()/atof() semantics: leading spaces ok, garbage -> 0.
    s.trim().parse::<i64>().unwrap_or(0)
}

fn parse_f64(s: &str) -> f64 {
    s.trim().parse::<f64>().unwrap_or(0.0)
}

struct Opts {
    url: Option<String>,
    mnt: Option<String>,
    custom_uid: i64,
    custom_gid: i64,
    allow_other_own_ids: bool,
    default_permissions: bool,
    multithreads: bool,
    fsname_given: bool,
    subtype_given: bool,
    database_type: String,
    database: String,
    table: String,
    cache_memory: i64,
    whitelist_cache: bool,
    nfs_connections: usize,
    your_key: String,
    fuse_opts: Vec<String>,
    unknown: Option<String>,
}

fn build_fuse_args(o: &Opts) -> Vec<String> {
    // mirrors the yourfs_argv[] assembly of the C version
    let mut args: Vec<String> = Vec::new();
    args.push("yourfs".to_string());
    args.push(o.mnt.clone().unwrap_or_default());
    args.extend(o.fuse_opts.iter().cloned());

    if o.allow_other_own_ids && !o.fuse_opts.iter().any(|a| a == "-oallow_other") {
        args.push("-oallow_other".to_string());
    }
    if !o.fsname_given {
        args.push(format!("-ofsname={}", o.url.clone().unwrap_or_default()));
    }
    if !o.subtype_given {
        args.push("-osubtype=fuse-nfs".to_string());
    }
    if o.default_permissions {
        args.push("-odefault_permissions".to_string());
    }
    if !o.multithreads {
        args.push("-s".to_string());
    }
    args
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut o = Opts {
        url: None,
        mnt: None,
        custom_uid: -1,
        custom_gid: -1,
        allow_other_own_ids: false,
        default_permissions: true,
        multithreads: true,
        fsname_given: false,
        subtype_given: false,
        database_type: String::new(),
        database: String::new(),
        table: String::new(),
        cache_memory: 10000,
        whitelist_cache: false,
        nfs_connections: nfscli::DEFAULT_POOL_SIZE,
        your_key: String::from("FADEP8"),
        fuse_opts: Vec::new(),
        unknown: None,
    };

    let mut p = cli::Parser::new(cli::YOURFS_SPECS, &argv);
    while let Some(cmd) = p.next_command() {
        match cmd {
            cli::Command::Opt(c, v) => {
                let v = v.unwrap_or_default();
                match c {
                    '?' => {
                        print_usage();
                        return;
                    }
                    'n' => o.url = Some(v),
                    'm' => {
                        o.mnt = Some(v);
                        o.fuse_opts.push("-oro".to_string());
                    }
                    'U' => o.custom_uid = parse_i64(&v),
                    'G' => o.custom_gid = parse_i64(&v),
                    'o' => o.allow_other_own_ids = true,
                    'a' => o.fuse_opts.push("-oallow_other".to_string()),
                    'u' => o.fuse_opts.push(format!("-ouid={v}")),
                    'g' => o.fuse_opts.push(format!("-ogid={v}")),
                    'D' => o.fuse_opts.push("-odebug".to_string()),
                    'p' => o.default_permissions = parse_i64(&v) != 0,
                    't' => o.multithreads = parse_i64(&v) != 0,
                    'd' => o.fuse_opts.push("-odirect_io".to_string()),
                    'r' => o.fuse_opts.push("-oallow_root".to_string()),
                    'k' => o.fuse_opts.push("-okernel_cache".to_string()),
                    'c' => o.fuse_opts.push("-oauto_cache".to_string()),
                    'l' => o.fuse_opts.push("-olarge_read".to_string()),
                    'h' => o.fuse_opts.push("-ohard_remove".to_string()),
                    'f' => {
                        o.fsname_given = true;
                        o.fuse_opts.push(format!("-ofsname={v}"));
                    }
                    's' => {
                        o.subtype_given = true;
                        o.fuse_opts.push(format!("-osubtype={v}"));
                    }
                    'b' => o.fuse_opts.push("-oblkdev".to_string()),
                    'i' => o.fuse_opts.push("-ointr".to_string()),
                    'R' => o.fuse_opts.push(format!("-omax_read={v}")),
                    'H' => o.fuse_opts.push(format!("-omax_readahead={v}")),
                    'A' => o.fuse_opts.push("-oasync_read".to_string()),
                    'S' => o.fuse_opts.push("-osync_read".to_string()),
                    'K' => o.fuse_opts.push(format!("-oumask={v}")),
                    'E' => o.fuse_opts.push(format!("-oentry_timeout={v}")),
                    'N' => o.fuse_opts.push(format!("-onegative_timeout={v}")),
                    'T' => o.fuse_opts.push(format!("-oattr_timeout={v}")),
                    'C' => o.fuse_opts.push(format!("-oac_attr_timeout={v}")),
                    'Y' => o.fuse_opts.push("-ononempty".to_string()),
                    'I' => o.fuse_opts.push(format!("-ointr_signal={v}")),
                    'q' => o.fuse_opts.push("-ouse_ino".to_string()),
                    'Q' => o.fuse_opts.push("-oreaddir_ino".to_string()),
                    'O' => o.fuse_opts.push("-oro".to_string()),
                    'X' => o.database = v,
                    'Z' => o.table = v,
                    'M' => o.cache_memory = (parse_f64(&v) * 1000.0) as i64,
                    'w' => o.whitelist_cache = true,
                    'V' => o.nfs_connections = parse_i64(&v).clamp(1, 64) as usize,
                    'F' => o.database_type = v,
                    'P' => {} // accepted like the C version; PASSWD module off
                    'y' => o.your_key = v,
                    _ => {}
                }
            }
            cli::Command::Unknown(msg) => {
                o.unknown = Some(msg);
                break;
            }
        }
    }
    if let Some(msg) = &o.unknown {
        eprintln!("yourfs: unrecognized option {msg}");
    }
    let your_key = o.your_key.clone();

    let url = match &o.url {
        Some(u) => u.clone(),
        None => {
            eprintln!("-n was not specified.");
            print_usage();
            std::process::exit(10);
        }
    };
    if o.mnt.is_none() {
        eprintln!("-m was not specified.");
        print_usage();
        std::process::exit(10);
    }

    if !key::key_is_good(&your_key) {
        println!("    Please update yourfs key, connect hanjun@nao.cas.cn");
        std::process::exit(0);
    }

    if !matches!(o.database_type.as_str(), "sqlite3" | "postgres" | "redis") {
        eprintln!("Please use -F to setup the database type, support sqlite3, postgres, redis");
        std::process::exit(1);
    }
    if db::check_ident(&o.table).is_err() {
        eprintln!("invalid -Z table name: {:?}", o.table);
        std::process::exit(1);
    }

    let database = match db::open(&o.database_type, &o.database, o.cache_memory) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open database: {e}");
            std::process::exit(1);
        }
    };
    // Opt-in read-through cache: only for mounts where the whitelist is known
    // to stay static (see `db::CachingDb`).
    let database = if o.whitelist_cache {
        db::caching(database)
    } else {
        database
    };
    let ctx = fs::DbCtx {
        db: database,
        table: o.table.clone(),
    };

    // ---- libnfs init, mirroring the C order exactly ----
    let nfs = unsafe { libnfs::nfs_init_context() };
    if nfs.is_null() {
        eprintln!("Failed to init context");
        std::process::exit(10);
    }
    let curl = paths::to_cstring(&url);
    let urls = unsafe { libnfs::nfs_parse_url_dir(nfs, curl.as_ptr()) };
    if urls.is_null() {
        let err = unsafe {
            let p = libnfs::nfs_get_error(nfs);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        eprintln!("Failed to parse url : {err}");
        unsafe { libnfs::nfs_destroy_context(nfs) };
        std::process::exit(10);
    }
    // the C version overrode -U/-G with url parameters if present
    if let Some(i) = url.find("uid=") {
        o.custom_uid = parse_i64(&url[i + 4..]);
    }
    if let Some(i) = url.find("gid=") {
        o.custom_gid = parse_i64(&url[i + 4..]);
    }
    let server = unsafe { (*urls).server };
    let export = unsafe { (*urls).path };
    if server.is_null() || export.is_null() {
        eprintln!("Failed to mount nfs share : url is incomplete");
        unsafe {
            libnfs::nfs_destroy_url(urls);
            libnfs::nfs_destroy_context(nfs);
        }
        std::process::exit(10);
    }
    let ret = unsafe { libnfs::nfs_mount(nfs, server, export) };
    if ret != 0 {
        let err = unsafe {
            let p = libnfs::nfs_get_error(nfs);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        eprintln!("Failed to mount nfs share : {err}");
        unsafe {
            libnfs::nfs_destroy_url(urls);
            libnfs::nfs_destroy_context(nfs);
        }
        std::process::exit(if ret < 0 { -ret } else { ret } as i32);
    }
    // Copy server/export out of `urls` before destroying it: the pool needs
    // them to mount its additional contexts.
    let server_c = unsafe { std::ffi::CStr::from_ptr(server) }.to_owned();
    let export_c = unsafe { std::ffi::CStr::from_ptr(export) }.to_owned();
    unsafe { libnfs::nfs_destroy_url(urls) };

    // `nfs` is the already-mounted first context, now owned by the pool:
    // nfscli::init stores it in slot 0 and opens a few more alongside it so
    // FUSE worker threads can issue NFS calls in parallel. Do not destroy it
    // here; nfscli::shutdown() tears down every pooled context after the loop.
    nfscli::init(
        nfs,
        &server_c,
        &export_c,
        o.custom_uid as i32,
        o.custom_gid as i32,
        o.allow_other_own_ids,
        unsafe { libc::getuid() },
        unsafe { libc::getgid() },
        o.nfs_connections,
    );

    let fuse_args = build_fuse_args(&o);
    let mut argv_c: Vec<CString> = fuse_args.into_iter().map(|s| paths::to_cstring(&s)).collect();
    eprintln!("Starting fuse_main()");
    let ret = fs::run(&ctx, o.custom_uid as i32, o.custom_gid as i32, &mut argv_c);

    // Gracefully free every pooled NFS context (slot 0 owns `nfs`).
    nfscli::shutdown();
    std::process::exit(ret);
}

fn print_usage() {
    println!(
        "Yourfs Usage(v1.0, Rust rewrite)
\t [-?|--help] 

yourfs options : 
\t [-n share_point| The server export to be mounted] 
\t [-m mount_path|  The client mount path] 
fuse options (see man mount.fuse): 
\t [-p [0|1]|--default_permissions=[0|1]] 
\t [-t [0|1]| Multi-threaded by default (1)] 
\t [-a|--allow_other] 
\t [-d|--direct_io] 
\t [-k|--kernel_cache] 
\t [-c|--auto_cache] 
\t [-A|--async_read] 
\t [-S|--sync_read] 
\t [-Y|--nonempty | setup it, if mount path is not empty] 
database options : 
\t [-F database_type ]  It is used to filter database type to read.(sqlite3, postgres or redis)
\t [-X database ] 
\t\tsqlite3:  /path/data.db
\t\tpostgres: ip,port,user,passwd,dbname
\t\tredis:    redis://host:port/db  OR  ip,port[,db[,passwd]]  OR  host:port
\t [-Z tablename| the table name to read] 
\t [-M MB| default 10, only used to setup sqlite cache]
\t [-w|--whitelist_cache| cache the whitelist in memory (enable ONLY if it will not change during the mount)] 
\t [-V N|--nfs_connections N| number of NFS connections to open in parallel (default 4, 1 disables pooling)] 
\t [-y yourkey| license key] 
\t (If you have any question or bug report, please connect hanjun@nao.cas.cn) 
"
    );
}
