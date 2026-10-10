use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use forge_diagnostics::{ForgeDiagnostic, codes};
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
	("0001_initial", include_str!("../../migrations/0001_initial.sql")),
	("0002_feedback", include_str!("../../migrations/0002_feedback.sql")),
	("0003_test_stderr", include_str!("../../migrations/0003_test_stderr.sql")),
	("0004_telemetry", include_str!("../../migrations/0004_telemetry.sql")),
];

const FLUSH_INPUT_ROWS: usize = 4096;
const FLUSH_ACTION_ROWS: usize = 1024;

enum ActionWrite {
	Seen { cache_key: String, component: String, name: String, now: i64 },
	CacheHit { cache_key: String },
	Duration { cache_key: String, duration_ms: i64 },
}

#[derive(Default)]
struct Pending {
	inputs: Vec<(String, Vec<(String, String)>)>,
	input_rows: usize,
	actions: Vec<ActionWrite>,
}

impl Pending {
	fn is_empty(&self) -> bool {
		self.inputs.is_empty() && self.actions.is_empty()
	}
}

pub struct CacheDb {
	state: parking_lot::Mutex<DbState>,
}

struct DbState {
	conn: Connection,
	pending: Pending,
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
		let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;");
		migrate(&conn)?;
		Ok(Self {
			state: parking_lot::Mutex::new(DbState {
				conn,
				pending: Pending::default(),
			}),
		})
	}

	pub fn flush(&self) {
		let mut state = self.state.lock();
		flush_pending(&mut state);
	}

	pub fn record_action(&self, cache_key: &str, component: &str, name: &str) {
		let write = ActionWrite::Seen {
			cache_key: cache_key.to_string(),
			component: component.to_string(),
			name: name.to_string(),
			now: now_secs(),
		};
		let mut state = self.state.lock();
		state.pending.actions.push(write);
		if state.pending.actions.len() >= FLUSH_ACTION_ROWS {
			flush_pending(&mut state);
		}
	}

	pub fn record_test(&self, cache_key: &str, component: &str, verdict: &str, duration_ms: u128, stderr_tail: &str) {
		let now = now_secs();
		let flipped = self.has_opposite_verdict(component, verdict);
		let mut state = self.state.lock();
		flush_pending(&mut state);
		let conn = &mut state.conn;
		let _ = conn.execute(
			"INSERT INTO test_results(cache_key, component, verdict, duration_ms, created_at, flake_count, run_count)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 1)
             ON CONFLICT(cache_key) DO UPDATE SET
                verdict = ?3,
                duration_ms = ?4,
                stderr = ?7,
                run_count = run_count + 1,
                flake_count = flake_count + ?6",
			rusqlite::params![
				cache_key,
				component,
				verdict,
				duration_ms as i64,
				now,
				i64::from(flipped),
				stderr_tail
			],
		);
		let _ = conn.execute(
			"INSERT INTO test_history(component, cache_key, verdict, created_at) VALUES (?1, ?2, ?3, ?4)",
			rusqlite::params![component, cache_key, verdict, now],
		);
	}

	pub fn latest_test_rows(&self) -> Vec<(String, String, i64, String)> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		let conn = &state.conn;
		let mut statement = match conn.prepare(
			"SELECT component, verdict, duration_ms, stderr FROM test_results
             WHERE rowid IN (SELECT MAX(rowid) FROM test_results GROUP BY component)
             ORDER BY component",
		) {
			Ok(s) => s,
			Err(_) => return Vec::new(),
		};
		statement
			.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
			.map(|rows| rows.flatten().collect())
			.unwrap_or_default()
	}

	fn has_opposite_verdict(&self, component: &str, verdict: &str) -> bool {
		self.state
			.lock()
			.conn
			.query_row(
				"SELECT EXISTS(SELECT 1 FROM test_history WHERE component = ?1 AND verdict != ?2)",
				[component, verdict],
				|row| row.get::<_, i64>(0),
			)
			.map(|exists| exists == 1)
			.unwrap_or(false)
	}

	pub fn flake_report(&self) -> Vec<(String, i64, i64)> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		let conn = &state.conn;
		let mut statement = match conn.prepare(
			"SELECT component,
                    SUM(verdict = 'PASSED'),
                    COUNT(*)
             FROM test_history GROUP BY component ORDER BY component",
		) {
			Ok(s) => s,
			Err(_) => return Vec::new(),
		};
		statement
			.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
			.map(|rows| rows.flatten().collect())
			.unwrap_or_default()
	}

	pub fn record_action_inputs(&self, cache_key: &str, inputs: &[(String, String)]) {
		let mut state = self.state.lock();
		state.pending.inputs.push((cache_key.to_string(), inputs.to_vec()));
		state.pending.input_rows += inputs.len();
		if state.pending.input_rows >= FLUSH_INPUT_ROWS {
			flush_pending(&mut state);
		}
	}

	pub fn stored_inputs(&self, cache_key: &str) -> Vec<(String, String)> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		let conn = &state.conn;
		let mut statement = match conn.prepare("SELECT path, hash FROM action_inputs WHERE cache_key = ?1") {
			Ok(s) => s,
			Err(_) => return Vec::new(),
		};
		statement
			.query_map([cache_key], |row| Ok((row.get(0)?, row.get(1)?)))
			.map(|rows| rows.flatten().collect())
			.unwrap_or_default()
	}

	pub fn latest_action_key(&self, component: &str, name: &str) -> Option<String> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		state
			.conn
			.query_row(
				"SELECT cache_key FROM actions WHERE component = ?1 AND name = ?2
                 ORDER BY last_accessed DESC LIMIT 1",
				[component, name],
				|row| row.get(0),
			)
			.ok()
	}

	pub fn prior_test_verdict(&self, cache_key: &str) -> Option<String> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		state
			.conn
			.query_row("SELECT verdict FROM test_results WHERE cache_key = ?1", [cache_key], |row| {
				row.get(0)
			})
			.ok()
	}

	pub fn mark_cache_hit(&self, cache_key: &str) {
		let write = ActionWrite::CacheHit {
			cache_key: cache_key.to_string(),
		};
		let mut state = self.state.lock();
		state.pending.actions.push(write);
	}

	pub fn record_duration(&self, cache_key: &str, duration_ms: u128) {
		let write = ActionWrite::Duration {
			cache_key: cache_key.to_string(),
			duration_ms: duration_ms as i64,
		};
		let mut state = self.state.lock();
		state.pending.actions.push(write);
	}

	pub fn replace_graph(&self, nodes: &[(String, String)], edges: &[(String, String)]) {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		let conn = &mut state.conn;
		let _ = conn.execute_batch("DELETE FROM component_edges; DELETE FROM components;");
		for (label, kind) in nodes {
			let _ = conn.execute(
				"INSERT INTO components(label, kind) VALUES (?1, ?2)",
				rusqlite::params![label, kind],
			);
		}
		for (dependent, dependency) in edges {
			let _ = conn.execute(
				"INSERT INTO component_edges(dependent, dependency) VALUES (?1, ?2)",
				rusqlite::params![dependent, dependency],
			);
		}
	}

	pub fn slowest_actions(&self, limit: usize) -> Vec<(String, String, i64, f64, i64)> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		state
			.conn
			.prepare(
				"SELECT component, name, COUNT(*), AVG(duration_ms), MAX(duration_ms)
                 FROM actions WHERE duration_ms > 0
                 GROUP BY component, name ORDER BY MAX(duration_ms) DESC LIMIT ?1",
			)
			.and_then(|mut s| {
				s.query_map([limit as i64], |row| {
					Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
				})
				.map(|rows| rows.flatten().collect())
			})
			.unwrap_or_default()
	}

	pub fn hit_rates(&self) -> Vec<(String, String, i64, i64)> {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		state
			.conn
			.prepare(
				"SELECT component, name, SUM(was_hit), COUNT(*)
                 FROM actions GROUP BY component, name ORDER BY component",
			)
			.and_then(|mut s| {
				s.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
					.map(|rows| rows.flatten().collect())
			})
			.unwrap_or_default()
	}

	pub fn wipe_test_results(&self) {
		let mut state = self.state.lock();
		flush_pending(&mut state);
		let _ = state.conn.execute("DELETE FROM test_results", []);
	}
}

impl Drop for CacheDb {
	fn drop(&mut self) {
		flush_pending(self.state.get_mut());
	}
}

fn flush_pending(state: &mut DbState) {
	if state.pending.is_empty() {
		return;
	}
	let pending = std::mem::take(&mut state.pending);
	let conn = &mut state.conn;
	let _ = conn.execute_batch("BEGIN");
	if let Ok(mut statement) = conn.prepare("INSERT INTO action_inputs(cache_key, path, hash) VALUES (?1, ?2, ?3)") {
		for (cache_key, rows) in &pending.inputs {
			let _ = conn.execute("DELETE FROM action_inputs WHERE cache_key = ?1", [cache_key]);
			for (path, hash) in rows {
				let _ = statement.execute(rusqlite::params![cache_key, path, hash]);
			}
		}
	}
	for write in &pending.actions {
		match write {
			ActionWrite::Seen {
				cache_key,
				component,
				name,
				now,
			} => {
				let _ = conn.execute(
					"INSERT INTO actions(cache_key, component, name, created_at, last_accessed)
                     VALUES (?1, ?2, ?3, ?4, ?4)
                     ON CONFLICT(cache_key) DO UPDATE SET last_accessed = ?4",
					rusqlite::params![cache_key, component, name, now],
				);
			}
			ActionWrite::CacheHit { cache_key } => {
				let _ = conn.execute("UPDATE actions SET was_hit = 1 WHERE cache_key = ?1", [cache_key]);
			}
			ActionWrite::Duration { cache_key, duration_ms } => {
				let _ = conn.execute(
					"UPDATE actions SET was_hit = 0, duration_ms = ?2 WHERE cache_key = ?1",
					rusqlite::params![cache_key, duration_ms],
				);
			}
		}
	}
	let _ = conn.execute_batch("COMMIT");
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
