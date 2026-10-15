//! `yourfs-tool` — builds whitelist databases: scans a local tree into a
//! `parent / name / realpath` text file, then loads that file into
//! sqlite3 or postgres. Rust rewrite of the C `yourfs_tool.c` main().

use std::io::BufRead;

use yourfs::{cli, db, scan};

struct ToolOpts {
    scan_path: String,
    output_filename: String,
    input_filename: String,
    database: String,
    table: String,
    cache_memory: i64,
    database_type: String,
    unknown: Option<String>,
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut o = ToolOpts {
        scan_path: String::new(),
        output_filename: "output.txt".to_string(),
        input_filename: "input.txt".to_string(),
        database: String::new(),
        table: "data".to_string(),
        cache_memory: 20000,
        database_type: String::new(),
        unknown: None,
    };

    if raw.is_empty() {
        print_usage();
        return;
    }

    let mut p = cli::Parser::new(cli::TOOL_SPECS, &raw);
    while let Some(cmd) = p.next_command() {
        match cmd {
            cli::Command::Opt(c, v) => {
                let v = v.unwrap_or_default();
                match c {
                    '?' => {
                        print_usage();
                        return;
                    }
                    's' => o.scan_path = v,
                    'o' => o.output_filename = v,
                    'i' => o.input_filename = v,
                    'X' => o.database = v,
                    'Z' => o.table = v,
                    'M' => o.cache_memory = (cli::parse_f64(&v) * 1000.0) as i64,
                    'F' => o.database_type = v,
                    'P' => {} // accepted like the C version; PASSWD module off
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
        eprintln!("yourfs_tool: unrecognized option {msg}");
    }

    // ---- scan mode: write the tree to a text file ----
    if !o.scan_path.is_empty() {
        if let Err(e) = scan::scan_to_file(&o.scan_path, &o.output_filename) {
            eprintln!("scan failed: {e}");
            std::process::exit(1);
        }
        return;
    }

    // ---- write mode: load the text file into the database ----
    if !matches!(o.database_type.as_str(), "sqlite3" | "postgres" | "redis") {
        return; // the C version silently fell through here
    }
    let file = match std::fs::File::open(&o.input_filename) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot open {}: {e}", o.input_filename);
            std::process::exit(1);
        }
    };
    let mut w: Box<dyn db::Writer> = match db::writer(&o.database_type, &o.database, o.cache_memory) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{} open failed: {e}", o.database_type);
            std::process::exit(1);
        }
    };
    if let Err(e) = w.create(&o.table) {
        eprintln!("create table failed: {e}");
        std::process::exit(1);
    }
    let reader = std::io::BufReader::new(file);
    let mut n = 0usize;
    let mut skipped = 0usize;
    for (ln, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        // Three columns per row; plain fields are whitespace-separated like
        // the C `fscanf("%s %s %s")` format, names containing whitespace are
        // double-quoted by `scan` (see `scan::parse_row`). A row that does
        // not tokenize to exactly three fields is corrupted input: skip and
        // count it rather than inserting mispaired columns.
        let cols = match scan::parse_row(&line) {
            Some(c) => c,
            None => {
                if !line.trim().is_empty() {
                    eprintln!("{}: malformed row on line {}, skipped", o.input_filename, ln + 1);
                    skipped += 1;
                }
                continue;
            }
        };
        if let Err(e) = w.insert(&o.table, &cols[0], &cols[1], &cols[2]) {
            eprintln!("insert failed at row {n}: {e}");
            std::process::exit(1);
        }
        n += 1;
    }
    if let Err(e) = w.finish() {
        eprintln!("commit failed: {e}");
        std::process::exit(1);
    }
    println!("{} rows written into table '{}' ({})", n, o.table, o.database_type);
    if skipped > 0 {
        println!("{skipped} malformed rows skipped");
    }
}

fn print_usage() {
    println!(
        "Yourfs_tool Usage(v1.0, Rust rewrite)
\t [-?|--help] 

yourfs_tool options : 
\t [-s scan_path] Setup path to scan. if not set, this routine used to write database. 
\t [-o txt name] The output txt-name when scan dir path. 

\t [-F database_type ] It is used to filter database type to write.(sqlite3, postgres or redis)
\t [-i txt name] The input txt-name when writing database. 
\t [-X database ] 
\t\tsqlite3:  /path/data.db
\t\tpostgres: ip,port,user,passwd,dbname
\t\tredis:    redis://host:port/db  OR  ip,port[,db[,passwd]]  OR  host:port
\t [-Z tablename ] 
\t [-M MB ] It is used to setup sqlite3 cache.
\t (If you have any question or bug report, please connect hanjun@nao.cas.cn) 
"
    );
}
