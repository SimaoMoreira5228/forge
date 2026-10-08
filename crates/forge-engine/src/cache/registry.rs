use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Mutex;

use forge_diagnostics::{ForgeDiagnostic, codes};
use sha2::Digest;

use crate::cache::Cas;
use crate::store::{Store, hasher};

pub struct Registry {
	url: String,
	index: Mutex<Option<BTreeMap<String, String>>>,
}

impl Registry {
	pub fn open(url: String) -> Self {
		Self {
			url: url.trim_end_matches('/').to_string(),
			index: Mutex::new(None),
		}
	}

	pub fn fetch(&self, key: &str, store: &Store, cas: &Cas) -> Result<bool, ForgeDiagnostic> {
		if cas.contains(key) {
			return Ok(true);
		}
		let Some(digest) = self.entry(key)? else {
			return Ok(false);
		};
		let Some(bytes) = self.download(&format!("{}/objects/{digest}", self.url))? else {
			return Ok(false);
		};
		if hasher::hex(&sha2::Sha256::digest(&bytes)) != digest {
			return Err(ForgeDiagnostic::error(
				codes::hermetic::HERMETIC_VIOLATION,
				format!("registry object `{digest}` does not match its digest"),
			));
		}
		let staging = store.staging("registry");
		std::fs::create_dir_all(&staging).map_err(|e| io(&staging, e))?;
		let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
		tar::Archive::new(decoder)
			.unpack(&staging)
			.map_err(|e| ForgeDiagnostic::error(8, format!("registry object `{digest}`: {e}")))?;
		{
			let _lock = store.lock("store")?;
			store.publish_dir(&staging, &cas.action_path(key))?;
		}
		Ok(true)
	}

	fn entry(&self, key: &str) -> Result<Option<String>, ForgeDiagnostic> {
		let mut index = self.index.lock().expect("registry index lock");
		if index.is_none() {
			*index = Some(self.load_index()?);
		}
		Ok(index.as_ref().and_then(|entries| entries.get(key).cloned()))
	}

	fn load_index(&self) -> Result<BTreeMap<String, String>, ForgeDiagnostic> {
		#[derive(serde::Deserialize)]
		struct Index {
			entries: Vec<Entry>,
		}
		#[derive(serde::Deserialize)]
		struct Entry {
			key: String,
			sha256: String,
		}
		let Some(bytes) = self.download(&format!("{}/index", self.url))? else {
			return Ok(BTreeMap::new());
		};
		let index: Index =
			serde_json::from_slice(&bytes).map_err(|e| ForgeDiagnostic::error(8, format!("registry index: {e}")))?;
		Ok(index.entries.into_iter().map(|entry| (entry.key, entry.sha256)).collect())
	}

	fn download(&self, url: &str) -> Result<Option<Vec<u8>>, ForgeDiagnostic> {
		let response = match ureq::get(url).call() {
			Ok(response) => response,
			Err(_) => return Ok(None),
		};
		let mut bytes = Vec::new();
		response
			.into_body()
			.into_reader()
			.read_to_end(&mut bytes)
			.map_err(|e| ForgeDiagnostic::error(8, format!("{url}: {e}")))?;
		Ok(Some(bytes))
	}
}

fn io(path: &std::path::Path, error: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;
	use std::io::{Read, Write};
	use std::net::TcpListener;

	use super::*;

	fn targz(dir: &std::path::Path) -> Vec<u8> {
		let mut buffer = Vec::new();
		{
			let encoder = flate2::write::GzEncoder::new(&mut buffer, flate2::Compression::fast());
			let mut builder = tar::Builder::new(encoder);
			builder.append_dir_all("", dir).unwrap();
			builder.into_inner().unwrap().finish().unwrap();
		}
		buffer
	}

	fn serve(index: String, objects: BTreeMap<String, Vec<u8>>) -> String {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let port = listener.local_addr().unwrap().port();
		std::thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let index = index.clone();
				let objects = objects.clone();
				std::thread::spawn(move || {
					let mut stream = stream;
					let mut request = [0u8; 2048];
					let read = stream.read(&mut request).unwrap_or(0);
					let request = String::from_utf8_lossy(&request[..read]);
					let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
					let body: Vec<u8> = if path == "/index" {
						index.into_bytes()
					} else if let Some(digest) = path.strip_prefix("/objects/") {
						objects.get(digest).cloned().unwrap_or_default()
					} else {
						Vec::new()
					};
					let header = format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
						body.len()
					);
					let _ = stream.write_all(header.as_bytes());
					let _ = stream.write_all(&body);
				});
			}
		});
		format!("http://127.0.0.1:{port}")
	}

	fn fixture(name: &str) -> (std::path::PathBuf, Store, Cas) {
		let root = std::env::temp_dir().join(format!("forge-registry-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(root.join("action/forge-out/bin")).unwrap();
		std::fs::write(root.join("action/manifest.json"), b"{}").unwrap();
		std::fs::write(root.join("action/forge-out/bin/app"), b"binary").unwrap();
		let store = Store::at(root.join("store"));
		let cas = Cas::at(store.actions());
		(root, store, cas)
	}

	#[test]
	fn imports_a_verified_object_into_the_cache() {
		let (root, store, cas) = fixture("ok");
		let tarball = targz(&root.join("action"));
		let digest = hasher::hex(&sha2::Sha256::digest(&tarball));
		let index = format!(r#"{{"entries":[{{"key":"abc","sha256":"{digest}"}}]}}"#);
		let registry = Registry::open(serve(index, BTreeMap::from([(digest, tarball)])));

		assert!(registry.fetch("abc", &store, &cas).unwrap());
		assert!(cas.contains("abc"));
		assert!(cas.action_path("abc").join("forge-out/bin/app").is_file());
		assert!(registry.fetch("abc", &store, &cas).unwrap(), "second fetch is a cache hit");
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn rejects_an_object_that_does_not_match_its_digest() {
		let (root, store, cas) = fixture("tamper");
		let tarball = targz(&root.join("action"));
		let key = "abc";
		let fake = "0".repeat(64);
		let index = format!(r#"{{"entries":[{{"key":"{key}","sha256":"{fake}"}}]}}"#);
		let registry = Registry::open(serve(index, BTreeMap::from([(fake, tarball)])));

		assert!(registry.fetch(key, &store, &cas).is_err());
		assert!(!cas.contains(key));
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn missing_entry_is_a_miss() {
		let (root, store, cas) = fixture("miss");
		let registry = Registry::open(serve(r#"{"entries":[]}"#.to_string(), BTreeMap::new()));
		assert!(!registry.fetch("abc", &store, &cas).unwrap());
		let _ = std::fs::remove_dir_all(&root);
	}
}
