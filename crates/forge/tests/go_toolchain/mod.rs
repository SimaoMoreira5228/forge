#![allow(dead_code)]

use std::path::Path;

pub fn link_go_toolchain(dir: &Path) -> bool {
	let store = forge_engine::store::store_root();
	for (name, version) in [("go", "1.27.1"), ("clang", "23.1.2")] {
		let source = store.join(format!("toolchains/{name}/{version}"));
		if !source.exists() {
			eprintln!("skipping: {name} {version} toolchain is not installed");
			return false;
		}
		let link = dir.join(format!(".forge/toolchains/{name}/{version}"));
		std::fs::create_dir_all(link.parent().unwrap()).unwrap();
		#[cfg(unix)]
		std::os::unix::fs::symlink(&source, &link).unwrap();
		#[cfg(not(unix))]
		super::rust_toolchain::copy_toolchain(&source, &link);
	}
	true
}
