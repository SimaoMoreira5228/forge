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
	copy_toolchain(&source, &link);
	true
}

#[cfg(not(unix))]
pub fn copy_toolchain(source: &Path, destination: &Path) {
	std::fs::create_dir_all(destination).unwrap();
	for entry in std::fs::read_dir(source).unwrap() {
		let entry = entry.unwrap();
		let target = destination.join(entry.file_name());
		if entry.path().is_dir() {
			copy_toolchain(&entry.path(), &target);
		} else if std::fs::hard_link(entry.path(), &target).is_err() {
			std::fs::copy(entry.path(), &target).unwrap();
		}
	}
}

#[cfg(all(test, not(unix)))]
#[test]
fn toolchain_fixture_materializes_nested_directories() {
	let root = std::env::temp_dir().join(format!("forge-toolchain-copy-{}", std::process::id()));
	let source = root.join("source");
	let destination = root.join("destination");
	std::fs::create_dir_all(source.join("bin")).unwrap();
	std::fs::write(source.join("bin/rustc.exe"), b"compiler").unwrap();
	copy_toolchain(&source, &destination);
	assert_eq!(std::fs::read(destination.join("bin/rustc.exe")).unwrap(), b"compiler");
	std::fs::remove_dir_all(root).unwrap();
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
