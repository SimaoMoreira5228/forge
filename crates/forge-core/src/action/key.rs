use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use blake3::Hasher;

pub fn compose_cache_key(
	runner_version: &str,
	profile_fingerprint: &str,
	toolchain_digest: Option<&str>,
	spec_fingerprint: [u8; 32],
	input_hashes: &BTreeMap<PathBuf, String>,
) -> String {
	let mut h = Hasher::new();
	put(&mut h, runner_version);
	put(&mut h, profile_fingerprint);
	put_opt(&mut h, toolchain_digest);
	h.update(&spec_fingerprint);
	for (path, digest) in input_hashes {
		put_path(&mut h, path);
		put(&mut h, digest);
	}
	h.finalize().to_hex().to_string()
}

fn put(h: &mut Hasher, s: &str) {
	h.update(&(s.len() as u64).to_le_bytes());
	h.update(s.as_bytes());
}

fn put_path(h: &mut Hasher, p: &Path) {
	put(h, &p.to_string_lossy());
}

fn put_opt(h: &mut Hasher, s: Option<&str>) {
	match s {
		Some(s) => put(h, s),
		None => {
			h.update(&[0u8]);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hashes() -> BTreeMap<PathBuf, String> {
		BTreeMap::from([(PathBuf::from("math.c"), "aa".repeat(32))])
	}

	#[test]
	fn key_stable_and_sensitive_to_every_part() {
		let key = |hashes: &BTreeMap<PathBuf, String>, tool: Option<&str>| {
			compose_cache_key("v1", "debug", tool, [7u8; 32], hashes)
		};
		let base = key(&hashes(), None);

		assert_eq!(base, key(&hashes(), None));

		let mut other_content = hashes();
		other_content.insert(PathBuf::from("math.c"), "bb".repeat(32));
		assert_ne!(base, key(&other_content, None));

		let mut more_inputs = hashes();
		more_inputs.insert(PathBuf::from("math.h"), "cc".repeat(32));
		assert_ne!(base, key(&more_inputs, None));

		assert_ne!(base, key(&hashes(), Some("digest")));
		assert_ne!(base, compose_cache_key("v2", "debug", None, [7u8; 32], &hashes()));
		assert_ne!(base, compose_cache_key("v1", "release", None, [7u8; 32], &hashes()));
	}
}
