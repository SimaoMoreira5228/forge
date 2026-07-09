use std::path::Path;
use std::process::Command;

fn forge_bin() -> &'static str {
	env!("CARGO_BIN_EXE_forge")
}

fn have_compiler() -> bool {
	Path::new("/usr/bin/cc").is_file() || Path::new("/usr/bin/gcc").is_file() || Path::new("/usr/bin/clang").is_file()
}

fn write_workspace(dir: &Path) {
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

fn run_forge(dir: &Path, args: &[&str]) -> (bool, String) {
	let out = Command::new(forge_bin())
		.args(args)
		.current_dir(dir)
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

#[test]
fn full_pipeline_build_run_and_cache_hits() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-itest-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "first build failed: {log}");

	let binary = dir.join("forge-out/bin/debug/app");
	let output = Command::new(&binary).output().expect("run built binary");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

	let (_, log2) = run_forge(&dir, &["build"]);
	assert!(log2.contains("4 cache hits"), "expected all hits, got: {log2}");

	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b + 1; }\n",
	)
	.unwrap();
	let (_, log3) = run_forge(&dir, &["build"]);
	assert!(log3.contains("3 executed") && log3.contains("1 cache hits"), "got: {log3}");

	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b; }\n",
	)
	.unwrap();
	let (_, log4) = run_forge(&dir, &["build"]);
	assert!(log4.contains("4 cache hits"), "time-travel hit expected, got: {log4}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn visibility_violations_fail_before_any_compilation() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-ivis-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("internal")).unwrap();
	std::fs::create_dir_all(dir.join("app")).unwrap();

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"vis\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("internal/FORGE.toml"),
		"[library.secret]\nvisibility = \"package\"\nsrcs = [\"secret.c\"]\n",
	)
	.unwrap();
	std::fs::write(dir.join("internal/secret.c"), "int secret(void) { return 1; }\n").unwrap();
	std::fs::write(
		dir.join("app/FORGE.toml"),
		"[binary.main]\ndeps = [\"//internal:secret\"]\nsrcs = []\n",
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "visibility violation must fail the build");
	assert!(log.contains("not permitted"), "got: {log}");
	assert!(
		!dir.join("forge-out/cas/actions").exists() || read_action_count(&dir) == 0,
		"no compilation may happen on graph errors"
	);

	let _ = std::fs::remove_dir_all(&dir);
}

fn read_action_count(dir: &Path) -> usize {
	std::fs::read_dir(dir.join("forge-out/cas/actions"))
		.map(|entries| entries.flatten().count())
		.unwrap_or(0)
}
