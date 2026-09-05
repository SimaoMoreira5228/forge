use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerProgram {
	pub command: String,
	pub variants: Vec<String>,
}

impl WorkerProgram {
	pub fn accepts(&self, variant: &str) -> bool {
		self.variants.is_empty() || self.variants.iter().any(|candidate| candidate == variant)
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerBinding {
	pub program: String,
	pub variant: String,
}

impl WorkerBinding {
	pub fn key(&self) -> String {
		format!("{}#{}", self.program, self.variant)
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerMount {
	pub source: PathBuf,
	pub target: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confinement {
	pub root: PathBuf,
	#[serde(default)]
	pub read_only: Vec<PathBuf>,
}

impl Confinement {
	pub fn new(root: impl Into<PathBuf>, read_only: Vec<PathBuf>) -> Self {
		Self {
			root: root.into(),
			read_only,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRequest {
	pub program: String,
	pub args: Vec<String>,
	pub env: BTreeMap<String, String>,
	pub workdir: PathBuf,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub mount: Option<WorkerMount>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub confinement: Option<Confinement>,
	#[serde(default)]
	pub outputs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkResponse {
	pub status: i32,
	pub stdout: Vec<u8>,
	pub stderr: Vec<u8>,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn request() -> WorkRequest {
		WorkRequest {
			program: "/toolchains/gcc/bin/gcc".into(),
			args: vec!["-c".into(), "src/math.c".into()],
			env: BTreeMap::from([("LANG".into(), "C.UTF-8".into())]),
			workdir: PathBuf::from("/out/exec/forge-out/obj"),
			mount: Some(WorkerMount {
				source: PathBuf::from("/out/sandbox/abc"),
				target: PathBuf::from("/out/exec"),
			}),
			confinement: Some(Confinement::new("/out/exec", vec![PathBuf::from("/toolchains/gcc/bin")])),
			outputs: vec![PathBuf::from("forge-out/obj/math.o")],
		}
	}

	#[test]
	fn requests_survive_the_wire() {
		let encoded = serde_json::to_vec(&request()).unwrap();
		assert_eq!(serde_json::from_slice::<WorkRequest>(&encoded).unwrap(), request());
	}

	#[test]
	fn a_confinement_travels_with_the_request_so_a_worker_confines_like_a_direct_spawn() {
		let expected = request().confinement;
		let encoded = serde_json::to_string(&request()).unwrap();
		assert!(encoded.contains("\"confinement\""), "{encoded}");
		let decoded: WorkRequest = serde_json::from_str(&encoded).unwrap();
		assert_eq!(decoded.confinement, expected);
	}

	#[test]
	fn unconfined_requests_omit_the_confinement() {
		let mut request = request();
		request.confinement = None;
		let encoded = serde_json::to_string(&request).unwrap();
		assert!(!encoded.contains("confinement"), "{encoded}");
		assert_eq!(serde_json::from_str::<WorkRequest>(&encoded).unwrap(), request);
	}

	#[test]
	fn unmounted_requests_omit_the_mount() {
		let mut request = request();
		request.mount = None;
		let encoded = serde_json::to_string(&request).unwrap();
		assert!(!encoded.contains("mount"), "{encoded}");
		assert_eq!(serde_json::from_str::<WorkRequest>(&encoded).unwrap(), request);
	}

	#[test]
	fn a_request_only_has_to_name_what_the_tool_needs_to_run() {
		let minimal = r#"{"program":"/bin/tool","args":[],"env":{},"workdir":"/sandbox"}"#;
		let request: WorkRequest = serde_json::from_str(minimal).unwrap();
		assert!(request.mount.is_none() && request.confinement.is_none() && request.outputs.is_empty());
	}

	#[test]
	fn binary_streams_survive_the_wire() {
		let response = WorkResponse {
			status: 3,
			stdout: vec![0, 255, b'a'],
			stderr: b"boom".to_vec(),
		};
		let encoded = serde_json::to_vec(&response).unwrap();
		assert_eq!(serde_json::from_slice::<WorkResponse>(&encoded).unwrap(), response);
	}

	#[test]
	fn variants_are_open_unless_the_catalog_lists_them() {
		assert!(
			WorkerProgram {
				command: "w".into(),
				variants: Vec::new()
			}
			.accepts("release")
		);
		let listed = WorkerProgram {
			command: "w".into(),
			variants: vec!["release".into()],
		};
		assert!(listed.accepts("release"));
		assert!(!listed.accepts("debug"));
	}

	#[test]
	fn the_pool_key_separates_variants_of_one_program() {
		let mut binding = WorkerBinding {
			program: "/bin/w".into(),
			variant: "a".into(),
		};
		let first = binding.key();
		binding.variant = "b".into();
		assert_ne!(first, binding.key());
	}
}
