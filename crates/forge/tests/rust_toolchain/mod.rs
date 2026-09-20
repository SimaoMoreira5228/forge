#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub fn have_rustc() -> bool {
	Command::new("rustc").arg("--version").output().is_ok()
}

pub fn link_rust_toolchain(dir: &Path) -> bool {
	let source = forge_engine::store::store_root().join("toolchains/rust/1.98.0");
	if !source.exists() {
		return false;
	}
	let link = dir.join(".forge/toolchains/rust/1.98.0");
	std::fs::create_dir_all(link.parent().unwrap()).unwrap();
	#[cfg(unix)]
	std::os::unix::fs::symlink(&source, &link).unwrap();
	#[cfg(not(unix))]
	std::fs::copy(&source, &link).unwrap();
	true
}

pub fn install_rust_toolchain(dir: &Path) -> bool {
	if !have_rustc() {
		eprintln!("skipping: no rustc");
		return false;
	}
	if link_rust_toolchain(dir) {
		return true;
	}
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
	let source = root.join(".forge/toolchains/rust/1.98.0");
	if !source.exists() {
		eprintln!("skipping: Rust 1.98.0 toolchain is not installed");
		return false;
	}
	let link = dir.join(".forge/toolchains/rust/1.98.0");
	std::fs::create_dir_all(link.parent().unwrap()).unwrap();
	#[cfg(unix)]
	{
		std::os::unix::fs::symlink(source, link).unwrap();
		true
	}
	#[cfg(not(unix))]
	{
		false
	}
}
