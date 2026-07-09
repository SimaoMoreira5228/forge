use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use blake3::Hasher;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputDeclaration {
	pub path: PathBuf,
	pub kind: OutputKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OutputKind {
	File,
	Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionSpec {
	pub name: String,
	pub component: String,
	pub command: String,
	pub args: Vec<String>,
	pub inputs: Vec<PathBuf>,
	pub outputs: Vec<OutputDeclaration>,
	pub env: BTreeMap<String, String>,
	pub toolchain_id: Option<String>,
}

impl ActionSpec {
	pub fn fingerprint(&self) -> [u8; 32] {
		let mut h = Hasher::new();
		put(&mut h, &self.name);
		put(&mut h, &self.component);
		put(&mut h, &self.command);
		for a in &self.args {
			put(&mut h, a);
		}
		for i in &self.inputs {
			put_path(&mut h, i);
		}
		for o in &self.outputs {
			put(&mut h, &o.path.to_string_lossy());
			put(
				&mut h,
				match o.kind {
					OutputKind::File => "file",
					OutputKind::Directory => "dir",
				},
			);
		}
		for (k, v) in &self.env {
			put(&mut h, k);
			put(&mut h, v);
		}
		put_opt(&mut h, self.toolchain_id.as_deref());
		h.finalize().into()
	}

	pub fn output_paths(&self) -> impl Iterator<Item = &Path> {
		self.outputs.iter().map(|o| o.path.as_path())
	}
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

	fn spec() -> ActionSpec {
		ActionSpec {
			name: "compile math.c".into(),
			component: "//lib:math".into(),
			command: "/toolchains/clang/bin/clang".into(),
			args: vec!["-c".into(), "math.c".into()],
			inputs: vec![PathBuf::from("math.c")],
			outputs: vec![OutputDeclaration {
				path: "math.o".into(),
				kind: OutputKind::File,
			}],
			env: BTreeMap::from([("CFLAGS".into(), "-O2".into())]),
			toolchain_id: Some("clang@19.1.7".into()),
		}
	}

	#[test]
	fn fingerprint_stable() {
		assert_eq!(spec().fingerprint(), spec().fingerprint());
	}

	#[test]
	fn fingerprint_changes_on_any_mutation() {
		let mut s2 = spec();
		s2.args.push("-Wall".into());
		assert_ne!(spec().fingerprint(), s2.fingerprint());

		let mut s3 = spec();
		s3.env.insert("LDFLAGS".into(), "-lm".into());
		assert_ne!(spec().fingerprint(), s3.fingerprint());

		let mut s4 = spec();
		s4.inputs.push(PathBuf::from("math.h"));
		assert_ne!(spec().fingerprint(), s4.fingerprint());

		let mut s5 = spec();
		s5.outputs[0].kind = OutputKind::Directory;
		assert_ne!(spec().fingerprint(), s5.fingerprint());
	}
}
