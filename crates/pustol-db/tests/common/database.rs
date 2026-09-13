//! A migrated database of a test's own, on the cluster the suites run against.
//!
//! Shared by the storage suite and the API suite through `#[path]`, so the two cannot drift on how a
//! database is named, made and cleared away.
//!
//! A test never drops its own database: its pool is still open when it returns, and a test that
//! panics returns nowhere. Every database is named after the second it was made instead, and the
//! first test of each process drops the ones that are long abandoned.
//!
//! Abandoned is judged by the cluster alone, never by which processes this machine can see: a suite
//! in another container or pid namespace is invisible from here, and its pid can be reused by an
//! unrelated process. A database is dropped only when it is older than any run takes and nobody is
//! connected to it, and without force, so a connection made in between keeps it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pustol_db::Store;

/// What every test database these suites make is named with, before the second, the pid and the
/// counter.
///
/// Names were once `pustol_t` followed by the numbers. No name in that form matches this prefix, so
/// the sweep never reads a second out of a name it did not make; `scripts/pg.sh start` clears both.
pub const PREFIX: &str = "pustol_test_";

/// How old a test database with nobody connected has to be before it counts as abandoned.
///
/// Far longer than a whole suite takes, so a database made a moment ago by a run that has not
/// connected to it yet is never taken for one left behind.
pub const ABANDONED_AFTER: Duration = Duration::from_mins(30);

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(1);
static SWEPT: AtomicBool = AtomicBool::new(false);

pub fn cluster_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/pustol".to_owned())
}

/// The same connection string pointed at another database on the same cluster.
fn pointing_at(url: &str, database: &str) -> String {
    let (base, query) = url
        .split_once('?')
        .map_or((url, ""), |(base, query)| (base, query));
    let stem = base.rsplit_once('/').map_or(base, |(stem, _)| stem);
    if query.is_empty() {
        format!("{stem}/{database}")
    } else {
        format!("{stem}/{database}?{query}")
    }
}

/// A pool on the cluster's maintenance database, where databases are made and dropped.
pub async fn maintenance() -> sqlx::PgPool {
    connect_to("postgres").await
}

/// A pool on one database of the cluster.
pub async fn connect_to(database: &str) -> sqlx::PgPool {
    let url = pointing_at(&cluster_url(), database);
    sqlx::PgPool::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("no cluster at {url}: {error}\nrun scripts/pg.sh start"))
}

/// Seconds since the epoch, the clock database names are written in.
pub fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The name of a test database under `prefix`, made at `made_at` by process `pid`, the `counter`th
/// it made.
pub fn database_name(prefix: &str, made_at: u64, pid: u32, counter: u64) -> String {
    format!("{prefix}{made_at}_{pid}_{counter}")
}

/// A migrated database of this test's own, with a pool belonging to this test's runtime.
pub async fn fresh_store() -> Store {
    let admin = maintenance().await;
    if !SWEPT.swap(true, Ordering::SeqCst) {
        sweep_abandoned(&admin, PREFIX, unix_seconds()).await;
    }
    let name = create_database(&admin, || {
        database_name(
            PREFIX,
            unix_seconds(),
            std::process::id(),
            NEXT_DATABASE.fetch_add(1, Ordering::Relaxed),
        )
    })
    .await;
    admin.close().await;

    let store = Store::connect(&pointing_at(&cluster_url(), &name), 12)
        .await
        .expect("the database just created accepts connections");
    store.migrate().await.expect("migrations apply");
    store
}

/// Creates a database under the first name `next` offers that nobody has taken, and gives the name.
///
/// The second, the pid and the counter keep the names of one machine apart, but two suites in
/// different pid namespaces can be the same pid in the same second. Creating a database either takes
/// the name or fails without touching anything, so a taken name is passed over for the next one.
pub async fn create_database(admin: &sqlx::PgPool, mut next: impl FnMut() -> String) -> String {
    loop {
        let name = next();
        assert!(is_plain(&name), "{name:?} is not a name this suite makes");
        // `create database` takes no bind parameters, so the name has to be interpolated. It is made
        // of letters, digits and underscores, as just asserted.
        match sqlx::query(sqlx::AssertSqlSafe(format!("create database \"{name}\"")))
            .execute(admin)
            .await
        {
            Ok(_) => return name,
            Err(error) if is_taken(&error) => {}
            Err(error) => panic!("cannot create {name}: {error}"),
        }
    }
}

/// Whether creating a database failed only because its name is taken. A concurrent creation under
/// the same name can report the catalogue's unique index instead of the name itself.
fn is_taken(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P04" || code == "23505")
}

/// Drops every database named under `prefix` that is older than [`ABANDONED_AFTER`] at `now` and
/// that nobody is connected to.
///
/// Without force: a connection made after the list was read makes the drop fail, and that database
/// is left for a later sweep.
pub async fn sweep_abandoned(admin: &sqlx::PgPool, prefix: &str, now: u64) {
    assert!(is_plain(prefix), "{prefix:?} is not a prefix this suite makes");
    let idle: Vec<String> = sqlx::query_scalar(
        "select datname from pg_database d
         where datname like $1
           and not exists (select 1 from pg_stat_activity a where a.datid = d.oid)",
    )
    .bind(format!("{}%", prefix.replace('_', "\\_")))
    .fetch_all(admin)
    .await
    .expect("the cluster lists its databases");
    for name in idle {
        let Some(made_at) = made_at(prefix, &name) else {
            continue;
        };
        if now.saturating_sub(made_at) < ABANDONED_AFTER.as_secs() {
            continue;
        }
        // Interpolated for the same reason as `create database`; `made_at` has admitted only the
        // prefix followed by numbers.
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "drop database if exists \"{name}\""
        )))
        .execute(admin)
        .await;
    }
}

/// The second a database was made, when `name` is exactly `prefix` and the three numbers
/// [`database_name`] writes; `None` for any other name.
fn made_at(prefix: &str, name: &str) -> Option<u64> {
    let mut numbers = name.strip_prefix(prefix)?.split('_');
    let (made_at, pid, counter) = (numbers.next()?, numbers.next()?, numbers.next()?);
    let is_number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if numbers.next().is_some() || ![made_at, pid, counter].into_iter().all(is_number) {
        return None;
    }
    made_at.parse().ok()
}

fn is_plain(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
