use blake3::{Hash, Hasher};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, Read, Result as IoResult};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HashResult {
	pub hash: String,
	pub algorithm: String,
	pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
	pub path: String,
	pub size: u64,
	pub hash: HashResult,
	pub chunk_hashes: Vec<String>,
}

pub struct CryptoUtils;

impl CryptoUtils {
	pub fn hash_bytes(data: &[u8]) -> HashResult {
		let hash = blake3::hash(data);
		HashResult {
			hash: hash.to_hex().to_string(),
			algorithm: "BLAKE3".to_string(),
			size: data.len() as u64,
		}
	}

	pub fn hash_string(data: &str) -> HashResult {
		Self::hash_bytes(data.as_bytes())
	}

	pub fn hash_file<P: AsRef<Path>>(path: P) -> IoResult<HashResult> {
		let file = File::open(&path)?;
		let mut reader = BufReader::new(file);
		let mut hasher = Hasher::new();
		let mut buffer = [0u8; 8192];
		let mut total_size = 0u64;

		loop {
			let bytes_read = reader.read(&mut buffer)?;
			if bytes_read == 0 {
				break;
			}
			hasher.update(&buffer[..bytes_read]);
			total_size += bytes_read as u64;
		}

		let hash = hasher.finalize();
		Ok(HashResult {
			hash: hash.to_hex().to_string(),
			algorithm: "BLAKE3".to_string(),
			size: total_size,
		})
	}

	pub fn analyze_file<P: AsRef<Path>>(path: P, chunk_size: usize) -> IoResult<FileMetadata> {
		let file = File::open(&path)?;
		let mut reader = BufReader::new(file);
		let mut main_hasher = Hasher::new();
		let mut chunk_hashes = Vec::new();
		let mut buffer = vec![0u8; chunk_size];
		let mut total_size = 0u64;

		loop {
			let bytes_read = reader.read(&mut buffer)?;
			if bytes_read == 0 {
				break;
			}

			let chunk = &buffer[..bytes_read];
			main_hasher.update(chunk);

			let chunk_hash = blake3::hash(chunk);
			chunk_hashes.push(chunk_hash.to_hex().to_string());

			total_size += bytes_read as u64;
		}

		let main_hash = main_hasher.finalize();
		Ok(FileMetadata {
			path: path.as_ref().to_string_lossy().to_string(),
			size: total_size,
			hash: HashResult {
				hash: main_hash.to_hex().to_string(),
				algorithm: "BLAKE3".to_string(),
				size: total_size,
			},
			chunk_hashes,
		})
	}

	pub fn verify_file_integrity<P: AsRef<Path>>(path: P, expected_metadata: &FileMetadata) -> IoResult<bool> {
		let current_metadata = Self::analyze_file(&path, 8192)?;
		Ok(current_metadata.hash == expected_metadata.hash && current_metadata.size == expected_metadata.size)
	}

	pub fn verify_chunks(chunk_hashes: &[String]) -> String {
		if chunk_hashes.is_empty() {
			return blake3::hash(b"").to_hex().to_string();
		}

		let combined: String = chunk_hashes.join("");
		blake3::hash(combined.as_bytes()).to_hex().to_string()
	}

	pub fn derive_key(password: &str, salt: &[u8], output_length: usize) -> Vec<u8> {
		let mut hasher = Hasher::new();
		hasher.update(salt);
		hasher.update(password.as_bytes());

		let mut output = vec![0u8; output_length];
		let mut reader = hasher.finalize_xof();
		reader.fill(&mut output);
		output
	}

	pub fn cas_path(hash: &str) -> String {
		if hash.len() < 4 {
			return format!("cas/{}", hash);
		}
		format!("cas/{}/{}/{}", &hash[0..2], &hash[2..4], hash)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_hash_string() {
		let result = CryptoUtils::hash_string("hello world");
		assert_eq!(result.algorithm, "BLAKE3");
		assert_eq!(result.size, 11);
		assert_eq!(
			result.hash,
			"d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
		);
	}

	#[test]
	fn test_derive_key() {
		let key = CryptoUtils::derive_key("password", b"salt", 32);
		assert_eq!(key.len(), 32);

		let key2 = CryptoUtils::derive_key("password", b"salt", 32);
		assert_eq!(key, key2);

		let key3 = CryptoUtils::derive_key("different", b"salt", 32);
		assert_ne!(key, key3);
	}

	#[test]
	fn test_cas_path() {
		let hash = "abcdef1234567890";
		let path = CryptoUtils::cas_path(hash);
		assert_eq!(path, "cas/ab/cd/abcdef1234567890");

		let short_hash = "ab";
		let short_path = CryptoUtils::cas_path(short_hash);
		assert_eq!(short_path, "cas/ab");
	}

	#[test]
	fn test_verify_chunks() {
		let chunks = vec!["hash1".to_string(), "hash2".to_string(), "hash3".to_string()];
		let merkle = CryptoUtils::verify_chunks(&chunks);
		assert!(!merkle.is_empty());

		let merkle2 = CryptoUtils::verify_chunks(&chunks);
		assert_eq!(merkle, merkle2);

		let mut chunks_reordered = chunks.clone();
		chunks_reordered.reverse();
		let merkle3 = CryptoUtils::verify_chunks(&chunks_reordered);
		assert_ne!(merkle, merkle3);
	}
}
