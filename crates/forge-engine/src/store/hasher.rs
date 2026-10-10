use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use parking_lot::RwLock;

pub fn hash_file(path: &Path) -> std::io::Result<[u8; 32]> {
	if path.is_dir() {
		return hash_tree(path, &|entry| hash_file(entry));
	}
	let mut file = std::fs::File::open(path)?;
	let mut hasher = blake3::Hasher::new();
	let mut buf = vec![0u8; 64 * 1024];
	loop {
		let read = file.read(&mut buf)?;
		if read == 0 {
			break;
		}
		hasher.update(&buf[..read]);
	}
	Ok(*hasher.finalize().as_bytes())
}

fn hash_tree(root: &Path, file: &dyn Fn(&Path) -> std::io::Result<[u8; 32]>) -> std::io::Result<[u8; 32]> {
	let mut entries = walkdir::WalkDir::new(root).into_iter().collect::<Result<Vec<_>, _>>()?;
	entries.sort_by_key(|entry| entry.path().to_path_buf());
	let mut hasher = blake3::Hasher::new();
	for entry in entries {
		if entry.file_type().is_file() {
			hasher.update(relative_to(root, entry.path()).to_string_lossy().as_bytes());
			hasher.update(&file(entry.path())?);
		}
	}
	Ok(*hasher.finalize().as_bytes())
}

fn relative_to(root: &Path, path: &Path) -> PathBuf {
	path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

type CachedHash = (u64, i64, [u8; 32]);

#[derive(Default)]
pub struct HashCache {
	files: RwLock<HashMap<PathBuf, CachedHash>>,
}

impl HashCache {
	pub fn new() -> Self {
		Self::default()
	}

	fn file(&self, path: &Path) -> std::io::Result<[u8; 32]> {
		let metadata = std::fs::metadata(path)?;
		let size = metadata.len();
		let mtime = metadata
			.modified()
			.ok()
			.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
			.map(|duration| duration.as_nanos() as i64)
			.unwrap_or(0);
		if let Some((cached_size, cached_mtime, hash)) = self.files.read().get(path)
			&& *cached_size == size
			&& *cached_mtime == mtime
		{
			return Ok(*hash);
		}
		let hash = hash_file(path)?;
		self.files.write().insert(path.to_path_buf(), (size, mtime, hash));
		Ok(hash)
	}

	pub fn path(&self, path: &Path) -> std::io::Result<[u8; 32]> {
		if !path.is_dir() {
			return self.file(path);
		}
		hash_tree(path, &|entry| self.file(entry))
	}
}

pub fn hash_inputs(
	workspace: &Path,
	inputs: &[PathBuf],
	cache: &HashCache,
) -> Result<BTreeMap<PathBuf, String>, std::io::Error> {
	let results: Vec<Result<(PathBuf, String), (PathBuf, std::io::Error)>> = inputs
		.iter()
		.map(|rel| {
			let abs = workspace.join(rel);
			let started = std::time::Instant::now();
			let result = match cache.path(&abs) {
				Ok(hash) => Ok((rel.clone(), hex(&hash))),
				Err(e) => Err((rel.clone(), e)),
			};
			let elapsed = started.elapsed();
			if elapsed.as_secs() >= 5 {
				eprintln!("slow input hash `{}`: {:.1}s", rel.display(), elapsed.as_secs_f64());
			}
			result
		})
		.collect();

	let mut out = BTreeMap::new();
	let mut first_error = None;
	for result in results {
		match result {
			Ok((path, hash)) => {
				out.insert(path, hash);
			}
			Err((path, e)) => {
				first_error.get_or_insert_with(|| {
					std::io::Error::new(e.kind(), format!("declared input `{}` is missing: {e}", path.display()))
				});
			}
		}
	}
	match first_error {
		Some(e) => Err(e),
		None => Ok(out),
	}
}

pub fn hash_path(root: &Path, path: &Path) -> std::io::Result<String> {
	let absolute = root.join(path);
	if absolute.is_dir() {
		let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&absolute)
			.into_iter()
			.filter_map(Result::ok)
			.filter(|entry| entry.file_type().is_file())
			.map(|entry| entry.path().to_path_buf())
			.collect();
		files.sort();
		let mut hasher = blake3::Hasher::new();
		for file in &files {
			hasher.update(relative_to(&absolute, file).to_string_lossy().as_bytes());
			hasher.update(&[0]);
			hasher.update(&hash_file(file)?);
		}
		return Ok(hex(hasher.finalize().as_bytes()));
	}
	Ok(hex(&hash_file(&absolute)?))
}

pub fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cache_reuses_unchanged_content() {
		let dir = std::env::temp_dir().join(format!("forge-hashcache-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let file = dir.join("a.txt");
		std::fs::write(&file, b"one").unwrap();
		let cache = HashCache::new();
		let first = cache.path(&file).unwrap();
		let second = cache.path(&file).unwrap();
		assert_eq!(first, second);
		std::fs::write(&file, b"changed content").unwrap();
		assert_ne!(first, cache.path(&file).unwrap());
		let _ = std::fs::remove_dir_all(&dir);
	}

	fn tree(root: &Path, contents: &[(&str, &[u8])]) -> PathBuf {
		let dir = root.to_path_buf();
		for (name, bytes) in contents {
			std::fs::create_dir_all(dir.join(name).parent().unwrap()).unwrap();
			std::fs::write(dir.join(name), bytes).unwrap();
		}
		dir
	}

	#[test]
	fn a_directory_hashes_by_its_layout_and_content_not_by_where_it_lives() {
		let base = std::env::temp_dir().join(format!("forge-hashtree-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&base);
		let first = tree(&base.join("one"), &[("out/x.rs", b"a"), ("out/y/z.rs", b"b")]);
		let second = tree(&base.join("two/deeper"), &[("out/x.rs", b"a"), ("out/y/z.rs", b"b")]);

		let cache = HashCache::new();
		assert_eq!(cache.path(&first).unwrap(), cache.path(&second).unwrap());
		assert_eq!(hash_file(&first).unwrap(), cache.path(&first).unwrap());

		std::fs::write(second.join("out/y/z.rs"), b"changed").unwrap();
		assert_ne!(cache.path(&first).unwrap(), cache.path(&second).unwrap());

		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn a_directory_rename_is_a_different_input() {
		let base = std::env::temp_dir().join(format!("forge-hashname-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&base);
		let first = tree(&base.join("one"), &[("out/x.rs", b"a")]);
		let second = tree(&base.join("two"), &[("renamed/x.rs", b"a")]);

		let cache = HashCache::new();
		assert_ne!(cache.path(&first).unwrap(), cache.path(&second).unwrap());

		let _ = std::fs::remove_dir_all(&base);
	}
}
