use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

pub fn hash_file(path: &Path) -> std::io::Result<[u8; 32]> {
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

pub fn hash_inputs(workspace: &Path, inputs: &[PathBuf]) -> Result<BTreeMap<PathBuf, String>, std::io::Error> {
	let hashes: Vec<Result<(PathBuf, String), (PathBuf, std::io::Error)>> = inputs
		.par_iter()
		.map(|rel| {
			let abs = workspace.join(rel);
			match hash_file(&abs) {
				Ok(h) => Ok((rel.clone(), hex(&h))),
				Err(e) => Err((rel.clone(), e)),
			}
		})
		.collect();

	let mut out = BTreeMap::new();
	let mut first_error = None;
	for result in hashes {
		match result {
			Ok((path, h)) => {
				out.insert(path, h);
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

pub fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hashing_is_content_sensitive_and_stable() {
		let tmp = std::env::temp_dir().join(format!("forge-hash-{}", std::process::id()));
		std::fs::create_dir_all(&tmp).unwrap();
		let a = tmp.join("a.txt");
		std::fs::write(&a, b"hello").unwrap();
		let h1 = hash_file(&a).unwrap();
		let h2 = hash_file(&a).unwrap();
		assert_eq!(h1, h2);

		std::fs::write(&a, b"hellp").unwrap();
		assert_ne!(hash_file(&a).unwrap(), h1);
		let _ = std::fs::remove_dir_all(&tmp);
	}

	#[test]
	fn missing_input_names_the_path() {
		let err = hash_inputs(Path::new("/nonexistent"), &[PathBuf::from("gone.c")]).unwrap_err();
		assert!(err.to_string().contains("`gone.c`"));
	}
}
