#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub fn have_rustc() -> bool {
	Command::new("rustc").arg("--version").output().is_ok()
}

pub fn forge_bin() -> &'static str {
	env!("CARGO_BIN_EXE_forge")
}

pub fn have_compiler() -> bool {
	Path::new("/usr/bin/cc").is_file() || Path::new("/usr/bin/gcc").is_file() || Path::new("/usr/bin/clang").is_file()
}

pub fn write_workspace(dir: &Path) {
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::create_dir_all(dir.join("lib")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"itest\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
        dir.join("FORGE.toml"),
        "[library.math]\nvisibility = \"public\"\nsrcs = [\"lib/math.c\"]\nhdrs = [\"lib/math.h\"]\nincludes = [\"lib\"]\n\n[binary.app]\ndeps = [\"math\"]\nsrcs = [\"src/main.c\"]\n",
    )
    .unwrap();
	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b; }\n",
	)
	.unwrap();
	std::fs::write(dir.join("lib/math.h"), "int add(int a, int b);\n").unwrap();
	std::fs::write(
		dir.join("src/main.c"),
		"#include <stdio.h>\n#include \"math.h\"\nint main(void) { printf(\"%d\\n\", add(20, 22)); return 0; }\n",
	)
	.unwrap();
}

pub fn run_forge(dir: &Path, args: &[&str]) -> (bool, String) {
	let out = Command::new(forge_bin())
		.args(args)
		.current_dir(dir)
		.env("FORGE_STORE_DIR", dir.join(".forge"))
		.output()
		.expect("launch forge");
	(
		out.status.success(),
		format!(
			"{}{}",
			String::from_utf8_lossy(&out.stdout),
			String::from_utf8_lossy(&out.stderr)
		),
	)
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
