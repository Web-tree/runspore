//! The store conformance suite against SQLite files; S01 and S03 race two instances
//! on one file, which is Q02.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use runspore_store_conformance::{store_conformance_tests, Fixture, StoreFactory};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Clock, Store};

struct SqliteFactory;

/// A path for a new database file, unique within this process and across processes.
pub fn fresh_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("runspore-store-sqlite-tests");
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(format!(
        "{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
}

impl StoreFactory for SqliteFactory {
    fn fresh(&self, clock: Arc<dyn Clock>) -> Fixture {
        let path = fresh_path();
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::open_with_clock(&path, clock).expect("open"));
        Fixture::new(
            store,
            Box::new(move |clock| {
                Arc::new(SqliteStore::open_with_clock(&path, clock).expect("reopen"))
            }),
        )
    }
}

store_conformance_tests!(SqliteFactory);
