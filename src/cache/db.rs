use rusqlite::{Connection, params};
use std::path::Path;

use crate::cache::Result;

const INIT_SCHEMA: &str = include_str!("migrations/001_initial.sql");
const TEST_RESULTS_SCHEMA: &str = include_str!("migrations/002_test_results.sql");

pub struct CacheDb {
	conn: Connection,
}

impl CacheDb {
	pub fn new(db_path: &Path) -> Result<Self> {
		if let Some(parent) = db_path.parent() {
			std::fs::create_dir_all(parent)?;
		}

		let conn = Connection::open(db_path)?;
		let db = Self { conn };
		db.init_tables()?;
		Ok(db)
	}

	fn init_tables(&self) -> Result<()> {
		self.conn.execute_batch(INIT_SCHEMA)?;
		self.conn.execute_batch(TEST_RESULTS_SCHEMA)?;
		Ok(())
	}

	pub fn record_artifact(&self, hash: &str, size: u64) -> Result<i64> {
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64;

		self.conn.execute(
			"INSERT OR REPLACE INTO artifacts (hash, size, created_at, last_accessed) VALUES (?1, ?2, ?3, ?3)",
			params![hash, size as i64, now],
		)?;

		Ok(self.conn.last_insert_rowid())
	}

	pub fn get_artifact(&self, hash: &str) -> Result<Option<ArtifactRecord>> {
		let mut stmt = self
			.conn
			.prepare("SELECT id, hash, size, created_at, last_accessed, compressed FROM artifacts WHERE hash = ?1")?;

		let mut rows = stmt.query(params![hash])?;

		if let Some(row) = rows.next()? {
			Ok(Some(ArtifactRecord {
				id: row.get(0)?,
				hash: row.get(1)?,
				size: row.get::<_, i64>(2)? as u64,
				created_at: row.get(3)?,
				last_accessed: row.get(4)?,
				compressed: row.get::<_, i32>(5)? != 0,
			}))
		} else {
			Ok(None)
		}
	}

	pub fn update_access_time(&self, hash: &str) -> Result<()> {
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64;

		self.conn
			.execute("UPDATE artifacts SET last_accessed = ?1 WHERE hash = ?2", params![now, hash])?;

		Ok(())
	}

	pub fn record_file(&self, path: &str, hash: &str, mtime: u64, artifact_id: i64) -> Result<()> {
		self.conn.execute(
			"INSERT OR REPLACE INTO files (path, hash, mtime, artifact_id) VALUES (?1, ?2, ?3, ?4)",
			params![path, hash, mtime as i64, artifact_id],
		)?;
		Ok(())
	}

	pub fn get_file_hash(&self, path: &str) -> Result<Option<(String, u64)>> {
		let mut stmt = self.conn.prepare("SELECT hash, mtime FROM files WHERE path = ?1")?;

		let mut rows = stmt.query(params![path])?;

		if let Some(row) = rows.next()? {
			Ok(Some((row.get(0)?, row.get::<_, i64>(1)? as u64)))
		} else {
			Ok(None)
		}
	}

	pub fn get_stats(&self) -> Result<CacheStats> {
		let total_files: i64 = self.conn.query_row("SELECT COUNT(*) FROM artifacts", [], |row| row.get(0))?;

		let total_size: i64 = self
			.conn
			.query_row("SELECT COALESCE(SUM(size), 0) FROM artifacts", [], |row| row.get(0))?;

		let oldest: Option<i64> = self
			.conn
			.query_row("SELECT MIN(created_at) FROM artifacts", [], |row| row.get(0))
			.ok();

		let newest: Option<i64> = self
			.conn
			.query_row("SELECT MAX(last_accessed) FROM artifacts", [], |row| row.get(0))
			.ok();

		Ok(CacheStats {
			total_files: total_files as u64,
			total_size: total_size as u64,
			oldest_timestamp: oldest,
			newest_timestamp: newest,
		})
	}

	pub fn list_artifacts(&self, limit: usize) -> Result<Vec<ArtifactRecord>> {
		let mut stmt = self.conn.prepare(
			"SELECT id, hash, size, created_at, last_accessed, compressed 
             FROM artifacts ORDER BY last_accessed DESC LIMIT ?1",
		)?;

		let records = stmt.query_map(params![limit as i64], |row| {
			Ok(ArtifactRecord {
				id: row.get(0)?,
				hash: row.get(1)?,
				size: row.get::<_, i64>(2)? as u64,
				created_at: row.get(3)?,
				last_accessed: row.get(4)?,
				compressed: row.get::<_, i32>(5)? != 0,
			})
		})?;

		let mut result = Vec::new();
		for record in records {
			result.push(record?);
		}
		Ok(result)
	}

	pub fn prune_orphaned(&self) -> Result<u64> {
		let deleted = self.conn.execute(
			"DELETE FROM artifacts WHERE id NOT IN (
                SELECT DISTINCT artifact_id FROM files WHERE artifact_id IS NOT NULL
            )",
			[],
		)?;

		Ok(deleted as u64)
	}

	pub fn migrate_from_json(&self, old_cache_path: &Path) -> Result<u64> {
		if !old_cache_path.exists() {
			return Ok(0);
		}

		// Check if migration was already applied (handle case where table doesn't exist yet)
		let already_migrated = self
			.conn
			.query_row(
				"SELECT COUNT(*) FROM schema_migrations WHERE name = ?1",
				params!["json_cache_migration"],
				|row| row.get::<_, i32>(0),
			)
			.unwrap_or(0);

		if already_migrated > 0 {
			log::debug!("JSON cache already migrated, skipping.");
			return Ok(0);
		}

		// Check if migration was already applied
		let already_migrated: Option<i32> = self
			.conn
			.query_row(
				"SELECT COUNT(*) FROM schema_migrations WHERE name = ?1",
				params!["json_cache_migration"],
				|row| row.get(0),
			)
			.ok();

		if already_migrated.map_or(false, |c| c > 0) {
			log::debug!("JSON cache already migrated, skipping.");
			return Ok(0);
		}

		let file = std::fs::File::open(old_cache_path)?;
		let reader = std::io::BufReader::new(file);

		let old_cache: crate::cache::BuildCache = match serde_json::from_reader(reader) {
			Ok(c) => c,
			Err(e) => {
				log::warn!("Failed to parse old cache: {}", e);
				return Ok(0);
			}
		};

		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64;

		let mut imported = 0u64;

		for entry in old_cache.file_hashes.iter() {
			let path = entry.key();
			let hash = entry.value();

			if let Ok(meta) = std::fs::metadata(path) {
				let size = meta.len() as i64;
				self.conn.execute(
					"INSERT OR REPLACE INTO artifacts (hash, size, created_at, last_accessed) VALUES (?1, ?2, ?3, ?3)",
					params![hash, size, now],
				)?;

				let artifact_id: i64 = self.conn.last_insert_rowid();

				let mtime = old_cache
					.mtimes
					.get(path)
					.and_then(|m| m.value().duration_since(std::time::UNIX_EPOCH).ok())
					.map(|d| d.as_secs() as i64)
					.unwrap_or(now);

				self.conn.execute(
					"INSERT OR REPLACE INTO files (path, hash, mtime, artifact_id) VALUES (?1, ?2, ?3, ?4)",
					params![path, hash, mtime, artifact_id],
				)?;
				imported += 1;
			}
		}

		for entry in old_cache.rule_hashes.iter() {
			let name = entry.key();
			let hash = entry.value();
			self.conn.execute(
				"INSERT OR IGNORE INTO rules (name, hash) VALUES (?1, ?2)",
				params![name, hash],
			)?;
			imported += 1;
		}

		// Record that migration was completed
		if imported > 0 {
			self.conn.execute(
				"INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
				params!["json_cache_migration", now],
			)?;
		}

		Ok(imported)
	}

	pub fn delete_old_cache(&self, old_cache_path: &Path) -> std::io::Result<()> {
		if old_cache_path.exists() {
			std::fs::remove_file(old_cache_path)?;
		}
		Ok(())
	}
}

#[derive(Debug)]
pub struct ArtifactRecord {
	pub id: i64,
	pub hash: String,
	pub size: u64,
	pub created_at: i64,
	pub last_accessed: i64,
	pub compressed: bool,
}

#[derive(Debug, Default)]
pub struct CacheStats {
	pub total_files: u64,
	pub total_size: u64,
	pub oldest_timestamp: Option<i64>,
	pub newest_timestamp: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TestResultRecord {
	pub id: i64,
	pub test_name: String,
	pub target: String,
	pub cache_key: String,
	pub verdict: String,
	pub duration_ms: i64,
	pub flake_count: i64,
	pub run_count: i64,
	pub stdout: Option<String>,
	pub stderr: Option<String>,
	pub created_at: i64,
}

impl CacheDb {
	pub fn record_test_result(
		&self,
		test_name: &str,
		target: &str,
		cache_key: &str,
		verdict: &str,
		duration_ms: i64,
		flake_count: i64,
		run_count: i64,
		stdout: Option<&str>,
		stderr: Option<&str>,
	) -> Result<i64> {
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64;

		self.conn.execute(
			"INSERT INTO test_results (test_name, target, cache_key, verdict, duration_ms, flake_count, run_count, stdout, stderr, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
			params![test_name, target, cache_key, verdict, duration_ms, flake_count, run_count, stdout, stderr, now],
		)?;

		Ok(self.conn.last_insert_rowid())
	}

	pub fn get_test_result(&self, cache_key: &str) -> Result<Option<TestResultRecord>> {
		let mut stmt = self.conn.prepare(
			"SELECT id, test_name, target, cache_key, verdict, duration_ms, flake_count, run_count, stdout, stderr, created_at FROM test_results WHERE cache_key = ?1"
		)?;

		let mut rows = stmt.query(params![cache_key])?;

		if let Some(row) = rows.next()? {
			Ok(Some(TestResultRecord {
				id: row.get(0)?,
				test_name: row.get(1)?,
				target: row.get(2)?,
				cache_key: row.get(3)?,
				verdict: row.get(4)?,
				duration_ms: row.get(5)?,
				flake_count: row.get(6)?,
				run_count: row.get(7)?,
				stdout: row.get(8)?,
				stderr: row.get(9)?,
				created_at: row.get(10)?,
			}))
		} else {
			Ok(None)
		}
	}

	pub fn get_failed_tests(&self) -> Result<Vec<TestResultRecord>> {
		let mut stmt = self.conn.prepare(
			"SELECT id, test_name, target, cache_key, verdict, duration_ms, flake_count, run_count, stdout, stderr, created_at FROM test_results WHERE verdict = 'FAILED' ORDER BY created_at DESC"
		)?;
		let mut rows = stmt.query([])?;
		let mut results = Vec::new();
		while let Some(row) = rows.next()? {
			results.push(TestResultRecord {
				id: row.get(0)?,
				test_name: row.get(1)?,
				target: row.get(2)?,
				cache_key: row.get(3)?,
				verdict: row.get(4)?,
				duration_ms: row.get(5)?,
				flake_count: row.get(6)?,
				run_count: row.get(7)?,
				stdout: row.get(8)?,
				stderr: row.get(9)?,
				created_at: row.get(10)?,
			});
		}
		Ok(results)
	}

	pub fn get_flake_report(&self) -> Result<Vec<TestResultRecord>> {
		let mut stmt = self.conn.prepare(
			"SELECT id, test_name, target, cache_key, verdict, duration_ms, flake_count, run_count, stdout, stderr, created_at FROM test_results WHERE flake_count > 0 ORDER BY flake_count DESC"
		)?;
		let mut rows = stmt.query([])?;
		let mut results = Vec::new();
		while let Some(row) = rows.next()? {
			results.push(TestResultRecord {
				id: row.get(0)?,
				test_name: row.get(1)?,
				target: row.get(2)?,
				cache_key: row.get(3)?,
				verdict: row.get(4)?,
				duration_ms: row.get(5)?,
				flake_count: row.get(6)?,
				run_count: row.get(7)?,
				stdout: row.get(8)?,
				stderr: row.get(9)?,
				created_at: row.get(10)?,
			});
		}
		Ok(results)
	}
}
