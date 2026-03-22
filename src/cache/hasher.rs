use std::fs::File;
use std::io::Read;
use std::path::Path;

use blake3::Hasher as Blake3Hasher;

const SMALL_FILE_THRESHOLD: u64 = 64 * 1024; // 64KB
const MEDIUM_FILE_THRESHOLD: u64 = 1024 * 1024; // 1MB

pub struct SmartHasher;

impl SmartHasher {
	pub fn hash_file(path: &Path) -> Result<String, std::io::Error> {
		let metadata = std::fs::metadata(path)?;
		let size = metadata.len();

		if size < SMALL_FILE_THRESHOLD {
			Self::hash_full(path)
		} else if size < MEDIUM_FILE_THRESHOLD {
			Self::hash_fast(path, &metadata)
		} else {
			Self::hash_sparse(path, &metadata)
		}
	}

	fn hash_full(path: &Path) -> Result<String, std::io::Error> {
		let mut file = File::open(path)?;
		let mut hasher = Blake3Hasher::new();
		let mut buffer = Vec::new();

		file.read_to_end(&mut buffer)?;
		hasher.update(&buffer);

		Ok(hasher.finalize().to_hex().to_string())
	}

	fn hash_fast(path: &Path, metadata: &std::fs::Metadata) -> Result<String, std::io::Error> {
		let mtime = metadata
			.modified()?
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.as_nanos())
			.unwrap_or(0);

		let size = metadata.len();

		let mut file = File::open(path)?;
		let mut hasher = Blake3Hasher::new();

		// Include size and mtime in hash
		hasher.update(&size.to_le_bytes());
		hasher.update(&mtime.to_le_bytes());

		// Hash first 4KB and last 4KB
		let boundary = 4 * 1024;

		let mut buffer = [0u8; 4096];

		// Read first boundary bytes
		let read = file.read(&mut buffer[..boundary])?;
		hasher.update(&buffer[..read]);

		// Read last boundary bytes if file is large enough
		if size > boundary as u64 * 2 {
			use std::io::Seek;
			let seek_pos = size - boundary as u64;
			file.seek(std::io::SeekFrom::Start(seek_pos))?;
			let read = file.read(&mut buffer[..boundary])?;
			hasher.update(&buffer[..read]);
		}

		Ok(hasher.finalize().to_hex().to_string())
	}

	fn hash_sparse(path: &Path, metadata: &std::fs::Metadata) -> Result<String, std::io::Error> {
		let mtime = metadata
			.modified()?
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.as_nanos())
			.unwrap_or(0);

		let size = metadata.len();

		let mut file = File::open(path)?;
		let mut hasher = Blake3Hasher::new();

		// Include size and mtime in hash
		hasher.update(&size.to_le_bytes());
		hasher.update(&mtime.to_le_bytes());

		// Sparse sampling: read 4KB at start, 25%, 50%, 75%, and end
		let samples = 5;
		let sample_size = 4 * 1024;
		let total_size = size as usize;

		let positions = if total_size > sample_size * samples {
			vec![
				0,
				total_size / 4,
				total_size / 2,
				(total_size * 3) / 4,
				total_size - sample_size,
			]
		} else {
			vec![0]
		};

		let mut buffer = vec![0u8; sample_size];

		use std::io::Seek;
		for pos in positions {
			file.seek(std::io::SeekFrom::Start(pos as u64))?;
			let read = file.read(&mut buffer)?;
			hasher.update(&buffer[..read]);
		}

		Ok(hasher.finalize().to_hex().to_string())
	}

	pub fn hash_string(data: &str) -> String {
		let mut hasher = Blake3Hasher::new();
		hasher.update(data.as_bytes());
		hasher.finalize().to_hex().to_string()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use tempfile::NamedTempFile;

	#[test]
	fn test_hash_small_file() {
		let mut file = NamedTempFile::new().unwrap();
		file.write_all(b"hello world").unwrap();

		let hash = SmartHasher::hash_file(file.path()).unwrap();
		assert!(!hash.is_empty());
	}

	#[test]
	fn test_hash_string() {
		let hash = SmartHasher::hash_string("hello world");
		assert_eq!(hash, "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24");
	}
}
