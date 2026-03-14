use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Target {
	pub name: String,
	pub triple: String,
	pub os: String,
	pub arch: String,
	pub abi: String,
}

impl Target {
	pub fn new(name: impl Into<String>, triple: impl Into<String>) -> Self {
		let name = name.into();
		let triple = triple.into();
		let parts: Vec<&str> = triple.split('-').collect();

		let (arch, os, abi) = if parts.len() >= 3 {
			let arch = parts[0].to_string();
			let os = if parts[1] == "apple" {
				"macos".to_string()
			} else {
				parts[2].split('.').next().unwrap_or("unknown").to_string()
			};
			let abi = parts.get(3).unwrap_or(&"unknown").to_string();
			(arch, os, abi)
		} else {
			("x86_64".to_string(), "linux".to_string(), "gnu".to_string())
		};

		Self {
			name,
			triple,
			os,
			arch,
			abi,
		}
	}

	pub fn is_native(&self) -> bool {
		let host_arch = if cfg!(target_arch = "x86_64") {
			"x86_64"
		} else if cfg!(target_arch = "aarch64") {
			"aarch64"
		} else {
			"unknown"
		};

		let host_os = if cfg!(target_os = "linux") {
			"linux"
		} else if cfg!(target_os = "windows") {
			"windows"
		} else if cfg!(target_os = "macos") {
			"macos"
		} else {
			"unknown"
		};

		self.arch == host_arch && self.os == host_os
	}

	pub fn executable_extension(&self) -> &'static str {
		if self.os == "windows" { ".exe" } else { "" }
	}

	pub fn static_lib_prefix(&self) -> &'static str {
		if self.os == "windows" { "" } else { "lib" }
	}

	pub fn static_lib_extension(&self) -> &'static str {
		if self.os == "windows" { ".lib" } else { ".a" }
	}

	pub fn dynamic_lib_extension(&self) -> &'static str {
		if self.os == "windows" {
			".dll"
		} else if self.os == "macos" {
			".dylib"
		} else {
			".so"
		}
	}
}

impl fmt::Display for Target {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.name)
	}
}

impl Default for Target {
	fn default() -> Self {
		let host = Self::host_target();
		Self::new(&host, &host)
	}
}

impl Target {
	pub fn host_target() -> String {
		let arch = if cfg!(target_arch = "x86_64") {
			"x86_64"
		} else if cfg!(target_arch = "aarch64") {
			"aarch64"
		} else if cfg!(target_arch = "arm") {
			"arm"
		} else {
			"unknown"
		};

		let os = if cfg!(target_os = "linux") {
			"linux"
		} else if cfg!(target_os = "windows") {
			"windows"
		} else if cfg!(target_os = "macos") {
			"macos"
		} else if cfg!(target_os = "freebsd") {
			"freebsd"
		} else {
			"unknown"
		};

		let abi = if cfg!(target_os = "windows") {
			if cfg!(target_env = "msvc") { "msvc" } else { "gnu" }
		} else if cfg!(target_os = "linux") {
			if cfg!(target_env = "musl") { "musl" } else { "gnu" }
		} else {
			"unknown"
		};

		format!("{}-unknown-{}-{}", arch, os, abi)
	}
}

pub fn predefined_targets() -> Vec<(&'static str, &'static str)> {
	vec![
		("linux_x64", "x86_64-unknown-linux-gnu"),
		("linux_x64_musl", "x86_64-unknown-linux-musl"),
		("linux_arm64", "aarch64-unknown-linux-gnu"),
		("windows_x64", "x86_64-pc-windows-msvc"),
		("windows_x64_gnu", "x86_64-pc-windows-gnu"),
		("macos_x64", "x86_64-apple-darwin"),
		("macos_arm64", "aarch64-apple-darwin"),
	]
}
