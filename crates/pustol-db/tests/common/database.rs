//! A migrated database of a test's own, on the cluster the suites run against.
//!
//! Shared by the storage suite and the API suite through `#[path]`, so the two cannot drift on how a
//! database is named, made and cleared away.
//!
//! A test never drops its own database: its pool is still open when it returns, and a test that
//! panics returns nowhere. Every database is named after the process that made it instead, and the
//! first test of each process drops the databases of processes that have ended, so a suite leaves
//! behind at most the databases of its own last run.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use pustol_db::Store;

const PREFIX: &str = "pustol_t";

static NEXT_DATABASE: AtomicI64 = AtomicI64::new(1);
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
    let url = pointing_at(&cluster_url(), "postgres");
    sqlx::PgPool::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("no cluster at {url}: {error}\nrun scripts/pg.sh start"))
}

/// A migrated database of this test's own, with a pool belonging to this test's runtime.
pub async fn fresh_store() -> Store {
    let admin = maintenance().await;
    if !SWEPT.swap(true, Ordering::SeqCst) {
        sweep_ended_runs(&admin).await;
    }
    let name = format!(
        "{PREFIX}{}_{}",
        std::process::id(),
        NEXT_DATABASE.fetch_add(1, Ordering::Relaxed)
    );
    // `create database` takes no bind parameters, so the name has to be interpolated. It is built
    // here from a process id and a counter and never from anything a caller supplies.
    sqlx::query(sqlx::AssertSqlSafe(format!("create database \"{name}\"")))
        .execute(&admin)
        .await
        .unwrap_or_else(|error| panic!("cannot create {name}: {error}"));
    admin.close().await;

    let store = Store::connect(&pointing_at(&cluster_url(), &name), 12)
        .await
        .expect("the database just created accepts connections");
    store.migrate().await.expect("migrations apply");
    store
}

/// Drops every test database whose process has ended.
///
/// A process is alive while `/proc/<pid>` exists. That sees only this machine's processes, so a
/// suite running in another container against the same cluster would look ended; the cluster
/// `scripts/pg.sh` starts is private to the machine that started it.
pub async fn sweep_ended_runs(admin: &sqlx::PgPool) {
    let names: Vec<String> =
        sqlx::query_scalar("select datname from pg_database where datname like 'pustol\\_t%'")
            .fetch_all(admin)
            .await
            .expect("the cluster lists its databases");
    for name in names {
        let Some(pid) = process_of(&name) else {
            continue;
        };
        if std::path::Path::new(&format!("/proc/{pid}")).exists() {
            continue;
        }
        // Interpolated for the same reason as `create database`; `process_of` has admitted only
        // names made of letters, digits and underscores.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "drop database if exists \"{name}\" with (force)"
        )))
        .execute(admin)
        .await
        .unwrap_or_else(|error| panic!("cannot drop {name}: {error}"));
    }
}

/// The process a test database was made by, or `None` for a name this suite never makes.
fn process_of(name: &str) -> Option<u32> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    name.strip_prefix(PREFIX)?.split_once('_')?.0.parse().ok()
}
