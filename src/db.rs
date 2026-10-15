//! Whitelist backends: the virtual directory tree exposed by yourfs is
//! defined by a `(parent, name, realpath)` table stored in sqlite3, postgres
//! or redis. `resolve()` maps a virtual path to its real path on the NFS
//! export (file isolation / renaming), `list()` enumerates one directory.

use std::collections::HashMap;
use std::error::Error;
use std::sync::{Condvar, Mutex};

use rusqlite::Connection;

use crate::paths;

/// A minimal fixed-capacity connection pool.
///
/// libfuse's high-level `fuse_loop_mt` releases its internal lock before
/// invoking our callbacks, so getattr/open/readdir/read run concurrently on
/// several worker threads. Giving those threads a single shared connection
/// guarded by one mutex just moves the serialization into the lock. This pool
/// hands out up to `max` owned connections, blocks a caller when all are in
/// use, and lazily creates more. It changes no query results — every operation
/// still hits the live database, so there is zero added staleness.
pub struct Pool<T: Send> {
    state: Mutex<PoolState<T>>,
    avail: Condvar,
    make: Box<dyn Fn() -> Result<T, String> + Send + Sync>,
    max: usize,
}

struct PoolState<T> {
    idle: Vec<T>,
    created: usize,
}

// `Pool<T>` is `Sync` automatically when `T: Send` (its only non-trivial field
// is `Mutex<PoolState<T>>`, which is `Sync` iff `PoolState<T>: Send`), so it
// can be shared across the FUSE worker threads with no unsafe.

impl<T: Send> Pool<T> {
    fn new(
        max: usize,
        make: impl Fn() -> Result<T, String> + Send + Sync + 'static,
    ) -> Self {
        Pool {
            state: Mutex::new(PoolState {
                idle: Vec::new(),
                created: 0,
            }),
            avail: Condvar::new(),
            make: Box::new(make),
            max: max.max(1),
        }
    }

    /// Check out a connection, creating one if the pool is below capacity and
    /// otherwise waiting for another thread to return one.
    fn get(&self) -> Result<Pooled<'_, T>, Box<dyn Error>> {
        let mut s = self.state.lock().unwrap();
        loop {
            if let Some(c) = s.idle.pop() {
                return Ok(Pooled {
                    pool: self,
                    conn: Some(c),
                    bad: false,
                });
            }
            if s.created < self.max {
                s.created += 1;
                drop(s);
                // Connect outside the lock so slow handshakes do not stall
                // other checkouts.
                match (self.make)() {
                    Ok(c) => {
                        return Ok(Pooled {
                            pool: self,
                            conn: Some(c),
                            bad: false,
                        })
                    }
                    Err(e) => {
                        let mut s = self.state.lock().unwrap();
                        s.created -= 1;
                        self.avail.notify_one();
                        return Err(e.into());
                    }
                }
            }
            s = self.avail.wait(s).unwrap();
        }
    }

    fn put(&self, c: T) {
        let mut s = self.state.lock().unwrap();
        s.idle.push(c);
        self.avail.notify_one();
    }

    /// Forget a connection that died (server restart, broken socket): drop it
    /// instead of returning it to the idle list, and free a slot so the next
    /// `get` can reconnect. Callers mark a checked-out guard bad via
    /// [`Pooled::mark_bad`]; its `Drop` lands here.
    fn discard(&self) {
        let mut s = self.state.lock().unwrap();
        s.created -= 1;
        self.avail.notify_one();
    }
}

/// RAII guard: returns the connection to its pool on drop, or discards it if
/// it was marked bad (a failed operation on a networked backend usually means
/// the connection is unusable, so we reconnect lazily on the next checkout).
pub struct Pooled<'a, T: Send> {
    pool: &'a Pool<T>,
    conn: Option<T>,
    bad: bool,
}

impl<T: Send> Pooled<'_, T> {
    /// Signal that the connection must not be reused (it errored mid-operation).
    pub fn mark_bad(&mut self) {
        self.bad = true;
    }
}

impl<T: Send> std::ops::Deref for Pooled<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.conn.as_ref().unwrap()
    }
}

impl<T: Send> std::ops::DerefMut for Pooled<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.conn.as_mut().unwrap()
    }
}

impl<T: Send> Drop for Pooled<'_, T> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            if self.bad {
                drop(c); // free the dead connection
                self.pool.discard();
            } else {
                self.pool.put(c);
            }
        }
    }
}

/// Default cap on simultaneous open connections / worker threads that can
/// touch the database at once.
const POOL_MAX: usize = 8;

/// Read-through memoization of the whitelist, opt-in via `-w`.
///
/// Only safe when the whitelist will NOT change for the lifetime of the mount
/// (the operator asserts that by enabling it): a revoked row or an edited
/// `realpath` would keep being served from the cache. With that caveat, it
/// removes every per-operation database round trip, which matters most for the
/// networked backends.
pub struct CachingDb {
    inner: Box<dyn Database>,
    paths: Mutex<HashMap<String, Option<String>>>,
    dirs: Mutex<HashMap<String, Vec<String>>>,
}

impl CachingDb {
    fn new(inner: Box<dyn Database>) -> Self {
        CachingDb {
            inner,
            paths: Mutex::new(HashMap::new()),
            dirs: Mutex::new(HashMap::new()),
        }
    }
}

impl Database for CachingDb {
    fn resolve(&self, table: &str, path: &str) -> Result<Option<String>, Box<dyn Error>> {
        if let Some(v) = self.paths.lock().unwrap().get(path) {
            return Ok(v.clone());
        }
        let v = self.inner.resolve(table, path)?;
        self.paths
            .lock()
            .unwrap()
            .insert(path.to_string(), v.clone());
        Ok(v)
    }

    fn list(&self, table: &str, path: &str) -> Result<Vec<String>, Box<dyn Error>> {
        if let Some(v) = self.dirs.lock().unwrap().get(path) {
            return Ok(v.clone());
        }
        let v = self.inner.list(table, path)?;
        self.dirs
            .lock()
            .unwrap()
            .insert(path.to_string(), v.clone());
        Ok(v)
    }
}

/// Shared by reference across the concurrent FUSE worker threads (via the
/// `DbPtr` static), so the trait must be `Sync`, not just `Send`. Without it
/// `&dyn Database` is not `Sync` and `CachingDb` (which owns a
/// `Box<dyn Database>`) would not be auto-`Sync` either.
pub trait Database: Send + Sync {
    /// Real path of the virtual `path`, or None when it is not whitelisted.
    fn resolve(&self, table: &str, path: &str) -> Result<Option<String>, Box<dyn Error>>;

    /// Names of the direct children of directory `path`, sorted like the
    /// original `... WHERE parent='<path>' order by name` queries.
    fn list(&self, table: &str, path: &str) -> Result<Vec<String>, Box<dyn Error>>;
}

/// Reject anything that is not a plain identifier before interpolating the
/// table name into SQL (the C version interpolated it blindly).
pub fn check_ident(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!("invalid table name: {name:?}"));
    }
    Ok(())
}

/// sqlite3 `CHAR(4096)/CHAR(256)` columns are space-padded; the C client
/// handed the padded values straight to fuse, we trim them instead.
fn trim_padded(s: String) -> String {
    s.trim_end_matches(' ').to_string()
}

pub struct SqliteDb {
    pool: Pool<Connection>,
}

impl SqliteDb {
    /// Open the db file and apply the cache_size pragma
    /// (`-M <MB>` -> `cache_size = MB*1000` KB pages, as in the C version).
    pub fn open(path: &str, cache_kb: i64) -> Result<Self, Box<dyn Error>> {
        // One eager open so a bad path surfaces at startup rather than on the
        // first request; the pool reconnects lazily with the same settings.
        let probe = Connection::open(path)?;
        probe.pragma_update(None, "cache_size", cache_kb)?;
        drop(probe);
        let path = path.to_string();
        let pool = Pool::new(POOL_MAX, move || {
            let c = Connection::open(&path).map_err(|e| e.to_string())?;
            c.pragma_update(None, "cache_size", cache_kb)
                .map_err(|e| e.to_string())?;
            Ok(c)
        });
        Ok(SqliteDb { pool })
    }
}

impl Database for SqliteDb {
    fn resolve(&self, table: &str, path: &str) -> Result<Option<String>, Box<dyn Error>> {
        if path == "/" {
            return Ok(Some("/".to_string()));
        }
        let conn = self.pool.get()?;
        let (parent, name) = crate::paths::split_path(path);
        let sql = format!(
            "SELECT realpath FROM {table} WHERE parent=?1 AND name=?2 LIMIT 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query_map([parent.as_str(), name.as_str()], |r| {
            let v: String = r.get(0)?;
            Ok(trim_padded(v))
        })?;
        match rows.next() {
            Some(v) => Ok(Some(v?)),
            None => Ok(None),
        }
    }

    fn list(&self, table: &str, path: &str) -> Result<Vec<String>, Box<dyn Error>> {
        let conn = self.pool.get()?;
        let sql = format!("SELECT name FROM {table} WHERE parent=?1 ORDER BY name");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([path], |r| {
            let v: String = r.get(0)?;
            Ok(trim_padded(v))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

/// Hands the (possibly multithreaded) FUSE callbacks a pooled synchronous
/// postgres client, so concurrent lookups no longer queue on one connection.
pub struct PgDb {
    pool: Pool<postgres::Client>,
}

impl PgDb {
    fn connect_one(
        host: &str,
        port: u16,
        user: &str,
        passwd: &str,
        dbname: &str,
    ) -> Result<postgres::Client, postgres::Error> {
        postgres::Config::new()
            .host(host)
            .port(port)
            .user(user)
            .password(passwd)
            .dbname(dbname)
            .connect(postgres::NoTls)
    }

    /// `-X ip,port,user,passwd,dbname`
    pub fn connect(spec: &str) -> Result<Self, Box<dyn Error>> {
        let parts: Vec<String> = spec.split(',').map(|s| s.to_string()).collect();
        if parts.len() != 5 {
            return Err("postgres -X expects ip,port,user,passwd,dbname".into());
        }
        let (host, port, user, pass, dbname) = (
            parts[0].clone(),
            parts[1].parse::<u16>()?,
            parts[2].clone(),
            parts[3].clone(),
            parts[4].clone(),
        );
        // Probe once so connection/auth errors surface at startup.
        Self::connect_one(&host, port, &user, &pass, &dbname)?;
        let (h, u, p, d) = (host, user, pass, dbname);
        let pool = Pool::new(POOL_MAX, move || {
            Self::connect_one(&h, port, &u, &p, &d).map_err(|e| e.to_string())
        });
        Ok(PgDb { pool })
    }
}

impl Database for PgDb {
    fn resolve(&self, table: &str, path: &str) -> Result<Option<String>, Box<dyn Error>> {
        if path == "/" {
            return Ok(Some("/".to_string()));
        }
        let mut client = self.pool.get()?;
        let (parent, name) = crate::paths::split_path(path);
        // char(n) comparisons in postgres work against the trimmed value.
        let sql = format!("SELECT realpath FROM {table} WHERE parent=$1 AND name=$2 LIMIT 1");
        let rows = match client.query(&sql, &[&parent.as_str(), &name.as_str()]) {
            Ok(rows) => rows,
            Err(e) => {
                // A query error can mean the TCP connection died (e.g. the
                // server restarted); discard so the next call reconnects.
                client.mark_bad();
                return Err(e.into());
            }
        };
        if rows.is_empty() {
            return Ok(None);
        }
        let v: Option<String> = rows[0].get(0);
        Ok(v.map(trim_padded))
    }

    fn list(&self, table: &str, path: &str) -> Result<Vec<String>, Box<dyn Error>> {
        let mut client = self.pool.get()?;
        let sql = format!("SELECT name FROM {table} WHERE parent=$1 ORDER BY name");
        let rows = match client.query(&sql, &[&path]) {
            Ok(rows) => rows,
            Err(e) => {
                client.mark_bad(); // dead connection: reconnect next time
                return Err(e.into());
            }
        };
        let mut out = Vec::new();
        for row in rows.iter() {
            let v: Option<String> = row.get(0);
            if let Some(v) = v {
                out.push(trim_padded(v));
            }
        }
        Ok(out)
    }
}

/// Factory mirroring the C `-F sqlite3|postgres` switch, extended with a
/// `redis` backend.
pub fn open(
    db_type: &str,
    database: &str,
    cache_kb: i64,
) -> Result<Box<dyn Database>, Box<dyn Error>> {
    match db_type {
        "sqlite3" => Ok(Box::new(SqliteDb::open(database, cache_kb)?)),
        "postgres" => Ok(Box::new(PgDb::connect(database)?)),
        "redis" => Ok(Box::new(RedisDb::connect(database)?)),
        other => Err(format!("unsupported database type: {other:?}").into()),
    }
}

/// Wrap a whitelist backend in the opt-in read-through cache (`-w`). Only
/// enable when the database will not change during the mount; see [`CachingDb`].
pub fn caching(db: Box<dyn Database>) -> Box<dyn Database> {
    Box::new(CachingDb::new(db))
}

/// Writer factory for `yourfs-tool`, mirroring [`open`].
pub fn writer(
    db_type: &str,
    database: &str,
    cache_kb: i64,
) -> Result<Box<dyn Writer>, Box<dyn Error>> {
    match db_type {
        "sqlite3" => Ok(Box::new(SqliteWriter::open(database, cache_kb)?)),
        "postgres" => Ok(Box::new(PgWriter::new(database)?)),
        "redis" => Ok(Box::new(RedisWriter::connect(database)?)),
        other => Err(format!("unsupported database type: {other:?}").into()),
    }
}

/// Whitelist table writer used by `yourfs-tool`. The tokio runtime for
/// postgres is created lazily so the sqlite path stays dependency-free
/// at runtime.
pub trait Writer {
    /// `DROP TABLE IF EXISTS <table>; CREATE TABLE …` like the C version.
    fn create(&mut self, table: &str) -> Result<(), Box<dyn Error>>;

    /// Insert one `(parent, name, realpath)` row inside the open batch.
    fn insert(&mut self, table: &str, parent: &str, name: &str, realpath: &str)
        -> Result<(), Box<dyn Error>>;

    fn finish(&mut self) -> Result<(), Box<dyn Error>>;
}

pub struct SqliteWriter {
    conn: Connection,
    table: String,
    rows: Vec<(String, String, String)>,
}

impl SqliteWriter {
    pub fn open(path: &str, cache_kb: i64) -> Result<Self, Box<dyn Error>> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "synchronous", "OFF")?;
        conn.pragma_update(None, "cache_size", cache_kb)?;
        Ok(SqliteWriter {
            conn,
            table: String::new(),
            rows: Vec::new(),
        })
    }
}

impl Writer for SqliteWriter {
    fn create(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        check_ident(table)?;
        self.table = table.to_string();
        self.conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS {table};
             CREATE TABLE {table} (id INTEGER PRIMARY KEY AUTOINCREMENT,parent CHAR(4096),name CHAR(256),realpath CHAR(4096));
             CREATE INDEX index_id ON {table}(id);
             CREATE INDEX index_parent ON {table}(parent);
             CREATE INDEX index_parent_name ON {table}(parent, name);"
        ))?;
        Ok(())
    }

    fn insert(
        &mut self,
        _table: &str,
        parent: &str,
        name: &str,
        realpath: &str,
    ) -> Result<(), Box<dyn Error>> {
        self.rows
            .push((parent.to_string(), name.to_string(), realpath.to_string()));
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        if self.rows.is_empty() {
            return Ok(());
        }
        // one transaction + reused statement, like the C tool's
        // sqlite3_prepare_v2 + BEGIN/COMMIT pattern (ids sequential from 1)
        self.conn.execute_batch("BEGIN")?;
        {
            let sql = format!(
                "INSERT INTO {}(id,parent,name,realpath) VALUES (?,?,?,?)",
                self.table
            );
            let mut stmt = self.conn.prepare(&sql)?;
            for (i, (parent, name, realpath)) in self.rows.iter().enumerate() {
                stmt.execute((i as i64 + 1, parent.as_str(), name.as_str(), realpath.as_str()))?;
            }
        }
        self.conn.execute_batch("COMMIT")?;
        self.rows.clear();
        Ok(())
    }
}

pub struct PgWriter {
    client: Option<postgres::Client>,
    spec: (String, String, String, String, String),
    table: String,
    rows: Vec<(String, String, String)>,
}

impl PgWriter {
    /// `-X ip,port,user,passwd,dbname`
    pub fn new(spec: &str) -> Result<Self, Box<dyn Error>> {
        let parts: Vec<&str> = spec.split(',').collect();
        if parts.len() != 5 {
            return Err("postgres -X expects ip,port,user,passwd,dbname".into());
        }
        Ok(PgWriter {
            client: None,
            spec: (
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
                parts[4].to_string(),
            ),
            table: String::new(),
            rows: Vec::new(),
        })
    }

    fn ensure(&mut self) -> Result<(), Box<dyn Error>> {
        if self.client.is_some() {
            return Ok(());
        }
        let (ip, port, user, passwd, dbname) = &self.spec;
        let client = postgres::Config::new()
            .host(ip)
            .port(port.parse::<u16>()?)
            .user(user)
            .password(passwd.as_str())
            .dbname(dbname)
            .connect(postgres::NoTls)?;
        self.client = Some(client);
        Ok(())
    }
}

impl Writer for PgWriter {
    fn create(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        check_ident(table)?;
        self.table = table.to_string();
        self.ensure()?;
        let c = self.client.as_mut().unwrap();
        // Multi-statement DDL must go through the simple query protocol;
        // `Client::query` uses the extended protocol and rejects `A; B`.
        c.simple_query(&format!(
            "DROP TABLE IF EXISTS {table};
                 CREATE TABLE {table} (id SERIAL primary key, parent VARCHAR(4096), name VARCHAR(256), realpath VARCHAR(4096));"
        ))?;
        println!("Table {table} has been created.");
        Ok(())
    }

    fn insert(
        &mut self,
        _table: &str,
        parent: &str,
        name: &str,
        realpath: &str,
    ) -> Result<(), Box<dyn Error>> {
        self.rows
            .push((parent.to_string(), name.to_string(), realpath.to_string()));
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        if self.rows.is_empty() {
            return Ok(());
        }
        self.ensure()?;
        let table = self.table.clone();
        let c = self.client.as_mut().unwrap();
        // batched inserts inside one transaction; semantically equal to the
        // C version's COPY ... FROM STDIN but portable across postgres crates
        c.execute("BEGIN", &[])?;
        {
            let sql =
                format!("INSERT INTO {table}(parent, name, realpath) VALUES ($1, $2, $3);");
            let stmt = c.prepare(&sql)?;
            for (parent, name, realpath) in &self.rows {
                c.execute(&stmt, &[&parent.as_str(), &name.as_str(), &realpath.as_str()])?;
            }
        }
        c.execute("COMMIT", &[])?;
        // Two DDL statements -> simple query protocol (see `create`).
        c.simple_query(&format!(
            "CREATE INDEX index_parent ON {table} USING hash(parent);
                 CREATE INDEX index_parent_name ON {table}(parent,name);"
        ))?;
        self.rows.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// redis backend
//
// The relational `(parent, name, realpath)` rows are mirrored into two native
// structures per whitelist table, keyed by a `{table}|` prefix:
//   * `{table}|d|<parent>` – a sorted set of child names (readdir; ZRANGE
//     returns them in byte order, matching `order by name`).
//   * `{table}|h|<parent>` – a hash mapping name -> realpath (resolve via
//     HGET, O(1)).
// ---------------------------------------------------------------------------

fn redis_url(spec: &str) -> String {
    if spec.contains("://") {
        return spec.to_string();
    }
    if !spec.contains(',') {
        // bare host, host:port or unix path fragment
        return format!("redis://{spec}/0");
    }
    // comma form, echoing the postgres `-X` style:
    //   ip,port[,db[,password]]
    let p: Vec<&str> = spec.split(',').collect();
    let host = p[0];
    let port = p.get(1).copied().unwrap_or("6379");
    let db = p.get(2).copied().unwrap_or("0");
    let pass = p.get(3).copied().unwrap_or("");
    if pass.is_empty() {
        format!("redis://{host}:{port}/{db}")
    } else {
        format!("redis://:{pass}@{host}:{port}/{db}")
    }
}

fn dir_key(table: &str, parent: &str) -> String {
    format!("{table}|d|{parent}")
}

fn hash_key(table: &str, parent: &str) -> String {
    format!("{table}|h|{parent}")
}

pub struct RedisDb {
    pool: Pool<redis::Connection>,
}

impl RedisDb {
    pub fn connect(spec: &str) -> Result<Self, Box<dyn Error>> {
        let url = redis_url(spec);
        // Probe once so an unreachable server / bad URL fails at startup.
        redis::Client::open(url.as_str())?
            .get_connection()
            .map_err(|e| -> Box<dyn Error> { e.into() })?;
        let pool = Pool::new(POOL_MAX, move || {
            let c = redis::Client::open(url.as_str()).map_err(|e| e.to_string())?;
            c.get_connection().map_err(|e| e.to_string())
        });
        Ok(Self { pool })
    }

    fn zrange(&self, key: &str) -> Result<Vec<String>, Box<dyn Error>> {
        let mut conn = self.pool.get()?;
        let r: Result<Vec<String>, redis::RedisError> = redis::cmd("ZRANGE")
            .arg(key)
            .arg(0)
            .arg(-1)
            .query(&mut *conn);
        match r {
            Ok(names) => Ok(names),
            Err(e) => {
                conn.mark_bad(); // dead socket: reconnect on next checkout
                Err(e.into())
            }
        }
    }
}

impl Database for RedisDb {
    fn resolve(&self, table: &str, path: &str) -> Result<Option<String>, Box<dyn Error>> {
        if path == "/" {
            return Ok(Some("/".to_string()));
        }
        let (parent, name) = paths::split_path(path);
        let mut conn = self.pool.get()?;
        let r: Result<Option<String>, redis::RedisError> = redis::cmd("HGET")
            .arg(hash_key(table, &parent))
            .arg(name)
            .query(&mut *conn);
        match r {
            Ok(rp) => Ok(rp),
            Err(e) => {
                conn.mark_bad(); // dead socket: reconnect on next checkout
                Err(e.into())
            }
        }
    }

    fn list(&self, table: &str, path: &str) -> Result<Vec<String>, Box<dyn Error>> {
        self.zrange(&dir_key(table, path))
    }
}

pub struct RedisWriter {
    conn: redis::Connection,
    table: String,
    rows: Vec<(String, String, String)>,
}

impl RedisWriter {
    pub fn connect(spec: &str) -> Result<Self, Box<dyn Error>> {
        let client = redis::Client::open(redis_url(spec))?;
        Ok(Self {
            conn: client.get_connection()?,
            table: String::new(),
            rows: Vec::new(),
        })
    }
}

impl Writer for RedisWriter {
    fn create(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        check_ident(table)?;
        self.table = table.to_string();
        // Drop every key belonging to this table via a SCAN/DEL loop (KEYS
        // would block the server on large keyspaces).
        let pattern = format!("{table}|*");
        let mut cursor: u64 = 0;
        loop {
            let (next, keys): (String, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(1000)
                .query(&mut self.conn)?;
            if !keys.is_empty() {
                let _: i64 = redis::cmd("DEL")
                    .arg(&keys)
                    .query(&mut self.conn)?;
            }
            cursor = next.parse()?;
            if cursor == 0 {
                break;
            }
        }
        Ok(())
    }

    fn insert(
        &mut self,
        _table: &str,
        parent: &str,
        name: &str,
        realpath: &str,
    ) -> Result<(), Box<dyn Error>> {
        self.rows
            .push((parent.to_string(), name.to_string(), realpath.to_string()));
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        if self.rows.is_empty() {
            return Ok(());
        }
        // One MULTI/EXEC transaction batching all ZADD + HSET writes.
        let mut pipe = redis::pipe();
        pipe.atomic();
        for (parent, name, realpath) in &self.rows {
            pipe.cmd("ZADD")
                .arg(dir_key(&self.table, parent))
                .arg(0)
                .arg(name);
            pipe.cmd("HSET")
                .arg(hash_key(&self.table, parent))
                .arg(name)
                .arg(realpath);
        }
        let _: () = pipe.query(&mut self.conn)?;
        self.rows.clear();
        Ok(())
    }
}
