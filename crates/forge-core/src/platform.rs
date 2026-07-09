use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Platform {
	pub os: String,
	pub arch: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub abi: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu: Option<String>,
}

impl Platform {
	pub fn host() -> Self {
		Self {
			os: std::env::consts::OS.to_string(),
			arch: normalize_arch(std::env::consts::ARCH),
			abi: host_abi(),
			cpu: None,
		}
	}

	pub fn catalog_key(&self) -> String {
		format!("{}-{}", self.os, self.arch)
	}

	pub fn matches(&self, predicate: &str) -> bool {
		let Some((field, value)) = predicate.split_once('=') else {
			return false;
		};
		match field.trim() {
			"os" => self.os == value,
			"arch" => self.arch == value,
			"abi" => self.abi.as_deref() == Some(value),
			"cpu" => self.cpu.as_deref() == Some(value),
			_ => false,
		}
	}
}

fn normalize_arch(arch: &str) -> String {
	match arch {
		"x86_64" | "x86" | "amd64" => "x86_64".into(),
		"aarch64" | "arm64" => "aarch64".into(),
		other => other.into(),
	}
}

fn host_abi() -> Option<String> {
	let s = std::env::consts::EXE_SUFFIX;
	if cfg!(target_env = "musl") {
		Some("musl".into())
	} else if cfg!(target_env = "gnu") {
		Some("gnu".into())
	} else {
		let _ = s;
		None
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigTransition {
	Host,
	Exec,
	Target,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn predicates_match_fields() {
		let p = Platform {
			os: "linux".into(),
			arch: "x86_64".into(),
			abi: Some("gnu".into()),
			cpu: None,
		};
		assert!(p.matches("os=linux"));
		assert!(p.matches("abi=gnu"));
		assert!(!p.matches("os=none"));
		assert!(!p.matches("bogus=x"));
	}

	#[test]
	fn catalog_key_shape() {
		let p = Platform {
			os: "linux".into(),
			arch: "x86_64".into(),
			abi: None,
			cpu: None,
		};
		assert_eq!(p.catalog_key(), "linux-x86_64");
	}
}
