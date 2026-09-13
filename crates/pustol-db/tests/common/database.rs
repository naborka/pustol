//! Per-test migrated database, shared by storage and API suites through `#[path]`.
//!
//! Test never drops own database: pool still open at return, and panicking test never returns.
//! Names carry creation second; first test of each process drops long-abandoned ones.
//!
//! Abandoned judged by cluster only, never by visible processes: suite in another container or pid
//! namespace is invisible, and pids get reused. Dropped only when older than any run and idle, and
//! without force, so connection made in between keeps it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pustol_db::Store;

pub const PREFIX: &str = "pustol_test_";

/// `pustol_t` is older form, numbers straight after. Name read only as exact prefix plus three
/// numbers, so forms never confused and nothing else under `pustol_t` read.
pub const SWEPT_PREFIXES: [&str; 2] = [PREFIX, "pustol_t"];

/// Far longer than whole suite, so fresh database not yet connected never counts as abandoned.
pub const ABANDONED_AFTER: Duration = Duration::from_mins(30);

/// `PostgreSQL` name limit in bytes; longer name silently truncated, so test would name no database.
const LONGEST_NAME: usize = 63;

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(1);
static SWEPT: AtomicBool = AtomicBool::new(false);

pub fn cluster_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/pustol".to_owned())
}

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

pub async fn maintenance() -> sqlx::PgPool {
    connect_to("postgres").await
}

pub async fn connect_to(database: &str) -> sqlx::PgPool {
    let url = pointing_at(&cluster_url(), database);
    sqlx::PgPool::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("no cluster at {url}: {error}\nrun scripts/pg.sh start"))
}

pub fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

pub fn database_name(prefix: &str, made_at: u64, pid: u32, counter: u64) -> String {
    format!("{prefix}{made_at}_{pid}_{counter}")
}

/// Pool bound to this test's runtime.
pub async fn fresh_store() -> Store {
    let admin = maintenance().await;
    if !SWEPT.swap(true, Ordering::SeqCst) {
        sweep_every(&admin, &SWEPT_PREFIXES, unix_seconds()).await;
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

/// Suites in different pid namespaces can share pid and second. Create either takes name or fails
/// harmlessly, so taken name skipped for next one `next` offers.
pub async fn create_database(admin: &sqlx::PgPool, mut next: impl FnMut() -> String) -> String {
    loop {
        let name = next();
        assert!(is_plain(&name), "{name:?} is not a name this suite makes");
        assert!(
            name.len() <= LONGEST_NAME,
            "{name:?} is longer than PostgreSQL keeps a name"
        );
        // `create database` takes no bind parameters; name asserted plain above.
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

/// Concurrent create of same name may report catalogue unique index `23505` instead of `42P04`.
fn is_taken(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "42P04" || code == "23505")
}

pub async fn sweep_every(admin: &sqlx::PgPool, prefixes: &[&str], now: u64) {
    for prefix in prefixes {
        sweep_abandoned(admin, prefix, now).await;
    }
}

/// Drops idle databases under `prefix` older than [`ABANDONED_AFTER`]. Without force: connection
/// made after listing makes drop fail, left for later sweep.
pub async fn sweep_abandoned(admin: &sqlx::PgPool, prefix: &str, now: u64) {
    assert!(
        is_plain(prefix),
        "{prefix:?} is not a prefix this suite makes"
    );
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
        // Interpolated like `create database`; `made_at` admitted only prefix plus numbers.
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "drop database if exists \"{name}\""
        )))
        .execute(admin)
        .await;
    }
}

/// Creation second, only when `name` is exactly `prefix` plus three numbers [`database_name`] writes.
pub fn made_at(prefix: &str, name: &str) -> Option<u64> {
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
