use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use forge_diagnostics::{ForgeDiagnostic, codes};
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[("0001_initial", include_str!("../migrations/0001_initial.sql"))];

pub struct CacheDb {
	conn: parking_lot::Mutex<Connection>,
}

impl CacheDb {
	pub fn open(out_dir: &Path) -> Result<Self, ForgeDiagnostic> {
		let dir = out_dir.join("cas");
		std::fs::create_dir_all(&dir).map_err(|e| {
			ForgeDiagnostic::error(
				codes::hermetic::HERMETIC_VIOLATION,
				format!("cannot create {}: {e}", dir.display()),
			)
		})?;
		let conn = Connection::open(dir.join("cache.db")).map_err(|e| db_err("open", e))?;
		migrate(&conn)?;
		Ok(Self {
			conn: parking_lot::Mutex::new(conn),
		})
	}

	pub fn record_action(&self, cache_key: &str, component: &str, name: &str) {
		let now = now_secs();
		let _ = self.conn.lock().execute(
			"INSERT INTO actions(cache_key, component, name, created_at, last_accessed)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(cache_key) DO UPDATE SET last_accessed = ?4",
			rusqlite::params![cache_key, component, name, now],
		);
	}

	pub fn record_test(&self, cache_key: &str, component: &str, verdict: &str, duration_ms: u128) {
		let _ = self.conn.lock().execute(
			"INSERT OR REPLACE INTO test_results(cache_key, component, verdict, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
			rusqlite::params![cache_key, component, verdict, duration_ms as i64, now_secs()],
		);
	}

	pub fn prior_test_verdict(&self, cache_key: &str) -> Option<String> {
		self.conn
			.lock()
			.query_row("SELECT verdict FROM test_results WHERE cache_key = ?1", [cache_key], |row| {
				row.get(0)
			})
			.ok()
	}

	pub fn wipe_test_results(&self) {
		let _ = self.conn.lock().execute("DELETE FROM test_results", []);
	}
}

fn now_secs() -> i64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs() as i64)
		.unwrap_or(0)
}

fn migrate(conn: &Connection) -> Result<(), ForgeDiagnostic> {
	conn.execute_batch("PRAGMA journal_mode = WAL;")
		.map_err(|e| db_err("migrate", e))?;
	conn.execute_batch(
		"CREATE TABLE IF NOT EXISTS _migrations (
             name       TEXT PRIMARY KEY,
             applied_at INTEGER NOT NULL
         );",
	)
	.map_err(|e| db_err("migrate", e))?;

	for (name, sql) in MIGRATIONS {
		let applied: i64 = conn
			.query_row("SELECT COUNT(*) FROM _migrations WHERE name = ?1", [name], |row| row.get(0))
			.map_err(|e| db_err("migrate", e))?;
		if applied > 0 {
			continue;
		}
		let record = format!("INSERT INTO _migrations(name, applied_at) VALUES('{name}', {});", now_secs());
		conn.execute_batch(&format!("BEGIN; {sql} {record} COMMIT;"))
			.map_err(|e| db_err(name, e))?;
	}
	Ok(())
}

fn db_err(stage: &str, e: rusqlite::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(
		codes::hermetic::HERMETIC_VIOLATION,
		format!("cache database {stage} failed: {e}"),
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn migrations_apply_once_and_are_idempotent() {
		let dir = std::env::temp_dir().join(format!("forge-db-mig-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();

		let db = CacheDb::open(&dir).unwrap();
		db.record_action("k1", "//a:b", "compile x");
		assert_eq!(db.prior_test_verdict("k1"), None);

		let conn = Connection::open(dir.join("cas/cache.db")).unwrap();
		let count: i64 = conn.query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0)).unwrap();
		assert_eq!(count, MIGRATIONS.len() as i64);
		drop(db);

		CacheDb::open(&dir).unwrap();
		let count: i64 = conn.query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0)).unwrap();
		assert_eq!(count, MIGRATIONS.len() as i64, "reopen must not reapply");

		let _ = std::fs::remove_dir_all(&dir);
	}
}
