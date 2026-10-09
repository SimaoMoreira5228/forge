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

	#[test]
	fn a_relocated_input_directory_keeps_its_key_while_a_renamed_one_loses_it() {
		let mut elsewhere = BTreeMap::new();
		elsewhere.insert(PathBuf::from("forge-out/build/debug/libc-0.2.189"), "dd".repeat(32));
		let mut elsewhere_renamed = BTreeMap::new();
		elsewhere_renamed.insert(PathBuf::from("forge-out/build/debug/libc-0.2.190"), "dd".repeat(32));

		let key = |inputs: &BTreeMap<PathBuf, String>| compose_cache_key("v1", "debug", None, [7u8; 32], inputs);
		assert_eq!(key(&elsewhere), key(&elsewhere.clone()));
		assert_ne!(key(&elsewhere), key(&elsewhere_renamed));
	}
}

#[cfg(test)]
mod discrimination {
	use std::collections::BTreeMap;
	use std::path::PathBuf;

	use super::compose_cache_key;
	use crate::action::spec::{ActionSpec, OutputDeclaration, OutputKind};
	use crate::platform::ConfigTransition;
	use crate::worker::WorkerBinding;

	const RUSTC_HERE: &str = "FORGE_TOOLCHAIN/rust@0123456789ab/bin/rustc";
	const RUSTC_THERE: &str = "FORGE_TOOLCHAIN/rust@0123456789ab/bin/rustdoc";
	const RUSTC_REBUILT: &str = "FORGE_TOOLCHAIN/rust@fedcba987654/bin/rustc";
	const CLANG: &str = "FORGE_TOOLCHAIN/clang@0123456789ab/bin/clang";

	fn spec(command: &str) -> ActionSpec {
		ActionSpec {
			name: "rustc //:forge -> forge-out/lib/debug/libforge.rlib".into(),
			component: "//:forge".into(),
			configuration: ConfigTransition::Target,
			command: command.into(),
			args: vec!["--crate-type".into(), "lib".into()],
			inputs: vec![PathBuf::from("src/lib.rs")],
			execution_deps: vec![],
			outputs: vec![OutputDeclaration {
				path: "forge-out/lib/debug/libforge.rlib".into(),
				kind: OutputKind::File,
			}],
			workdir: None,
			is_test: false,
			stdout: None,
			compile_command: None,
			environment_files: Vec::new(),
			argument_files: Vec::new(),
			env: BTreeMap::from([("CARGO_PKG_NAME".into(), "forge".into())]),
			toolchain_ids: Vec::new(),
			toolchain_id: Some("rust@0123456789ab".into()),
			worker: None,
		}
	}

	fn key(spec: &ActionSpec, profile: &str, digest: Option<&str>, inputs: &BTreeMap<PathBuf, String>) -> String {
		compose_cache_key("0.2.0", profile, digest, spec.fingerprint(), inputs)
	}

	fn inputs() -> BTreeMap<PathBuf, String> {
		BTreeMap::from([(PathBuf::from("src/lib.rs"), "aa".repeat(32))])
	}

	#[test]
	fn the_same_inputs_key_the_same_wherever_the_toolchain_is_installed() {
		let installed_here = key(&spec(RUSTC_HERE), "debug", Some("0123456789ab"), &inputs());
		assert_eq!(
			installed_here,
			key(&spec(RUSTC_HERE), "debug", Some("0123456789ab"), &inputs())
		);
		assert_eq!(
			installed_here,
			key(&spec(RUSTC_HERE), "debug", Some("0123456789ab"), &inputs())
		);
	}

	#[test]
	fn a_different_command_toolchain_configuration_worker_or_input_is_a_different_key() {
		let base = key(&spec(RUSTC_HERE), "debug", Some("0123456789ab"), &inputs());

		assert_ne!(
			base,
			key(&spec(RUSTC_THERE), "debug", Some("0123456789ab"), &inputs()),
			"another binary"
		);
		assert_ne!(
			base,
			key(&spec(CLANG), "debug", Some("0123456789ab"), &inputs()),
			"another toolchain"
		);
		assert_ne!(
			base,
			key(&spec(RUSTC_REBUILT), "debug", Some("fedcba987654"), &inputs()),
			"a rebuilt toolchain"
		);

		let mut different_args = spec(RUSTC_HERE);
		different_args.args.push("-C".into());
		assert_ne!(base, key(&different_args, "debug", Some("0123456789ab"), &inputs()));

		assert_ne!(
			base,
			key(&spec(RUSTC_HERE), "release", Some("0123456789ab"), &inputs()),
			"another profile"
		);
		assert_ne!(
			base,
			key(&spec(RUSTC_HERE), "debug", Some("fedcba987654"), &inputs()),
			"another toolchain digest"
		);

		let mut host = spec(RUSTC_HERE);
		host.configuration = ConfigTransition::Host;
		assert_ne!(base, key(&host, "debug", Some("0123456789ab"), &inputs()));

		let mut worker = spec(RUSTC_HERE);
		worker.worker = Some(WorkerBinding {
			program: RUSTC_HERE.into(),
			variant: "release".into(),
		});
		assert_ne!(base, key(&worker, "debug", Some("0123456789ab"), &inputs()));

		let mut other_env = spec(RUSTC_HERE);
		other_env.env.insert("CARGO_PKG_NAME".into(), "other".into());
		assert_ne!(base, key(&other_env, "debug", Some("0123456789ab"), &inputs()));

		let mut changed_input = inputs();
		changed_input.insert(PathBuf::from("src/lib.rs"), "bb".repeat(32));
		assert_ne!(base, key(&spec(RUSTC_HERE), "debug", Some("0123456789ab"), &changed_input));

		let mut added_input = inputs();
		added_input.insert(PathBuf::from("src/build.rs"), "cc".repeat(32));
		assert_ne!(base, key(&spec(RUSTC_HERE), "debug", Some("0123456789ab"), &added_input));
	}

	#[test]
	fn a_toolchain_id_without_a_configured_digest_still_keys_by_the_reference() {
		let mut unresolved = spec(RUSTC_HERE);
		unresolved.toolchain_id = None;
		let base = key(&unresolved, "debug", None, &inputs());
		assert_ne!(base, key(&spec(RUSTC_REBUILT), "debug", None, &inputs()));
	}
}
