mod forge_cli;

use forge_cli::run_forge;

#[test]
fn clang_cxx_modules_precompile_and_link() {
	let clang = std::path::Path::new("/usr/bin/clang++");
	if !clang.is_file() {
		eprintln!("skipping: no clang++");
		return;
	}
	let probe = std::env::temp_dir().join(format!("forge-imod-probe-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&probe);
	std::fs::create_dir_all(&probe).unwrap();
	std::fs::write(probe.join("probe.cppm"), "export module probe;\n").unwrap();
	let supported = std::process::Command::new(clang)
		.args([
			"-std=c++20",
			"-x",
			"c++-module",
			"--precompile",
			"probe.cppm",
			"-o",
			"probe.pcm",
		])
		.current_dir(&probe)
		.output()
		.is_ok_and(|output| output.status.success());
	let _ = std::fs::remove_dir_all(&probe);
	if !supported {
		eprintln!("skipping: clang++ without C++20 modules");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-imod-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::create_dir_all(dir.join("lib")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"mod\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.clang]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.cpp\", \"lib/core.cppm\"]\nincludes = [\"lib\"]\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("lib/core.cppm"),
		"export module math.core;\nexport int add(int a, int b) { return a + b; }\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/main.cpp"),
		"import math.core;\n#include <cstdio>\nint main() { std::printf(\"%d\\n\", add(20, 22)); }\n",
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "module build failed: {log}");
	let binary = dir.join("forge-out/bin/debug/app");
	let output = std::process::Command::new(&binary).output().expect("run built binary");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

	let (_, log2) = run_forge(&dir, &["build"]);
	assert!(log2.contains("0 executed"), "module build must be cached: {log2}");

	let _ = std::fs::remove_dir_all(&dir);
}
