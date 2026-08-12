mod common;

use common::*;

fn write_two_binaries(dir: &std::path::Path) {
	std::fs::create_dir_all(dir.join("lib")).unwrap();
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"alg\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[library.math]\nsrcs = [\"lib/math.c\"]\nhdrs = [\"lib/math.h\"]\nincludes = [\"lib\"]\n\n[binary.app]\ndeps = [\"math\"]\nsrcs = [\"src/app.c\"]\n\n[binary.tool]\ndeps = [\"math\"]\nsrcs = [\"src/tool.c\"]\n",
	)
	.unwrap();
	std::fs::write(dir.join("lib/math.c"), "int add(int a, int b) { return a + b; }\n").unwrap();
	std::fs::write(dir.join("lib/math.h"), "int add(int a, int b);\n").unwrap();
	let main = "#include <stdio.h>\n#include \"math.h\"\nint main(void) { printf(\"%d\\n\", add(1, 2)); return 0; }\n";
	std::fs::write(dir.join("src/app.c"), main).unwrap();
	std::fs::write(dir.join("src/tool.c"), main).unwrap();
}

#[test]
fn algebraic_selection_builds_only_the_matched_targets() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-ialg-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_two_binaries(&dir);

	let (ok, log) = run_forge(&dir, &["build", "//:tool"]);
	assert!(ok, "selected build failed: {log}");
	assert!(dir.join("forge-out/bin/debug/tool").exists(), "selection must build tool");
	assert!(
		!dir.join("forge-out/bin/debug/app").exists(),
		"unselected target must not be built"
	);

	let (ok, log) = run_forge(&dir, &["build", "kind(binary, //...)"]);
	assert!(ok, "query build failed: {log}");
	assert!(
		dir.join("forge-out/bin/debug/app").exists() && dir.join("forge-out/bin/debug/tool").exists(),
		"query selection must build both binaries"
	);

	let (ok, log) = run_forge(&dir, &["build", "//:nope"]);
	assert!(!ok, "unmatched selection must fail: {log}");
	assert!(log.contains("matched no components"), "{log}");

	let _ = std::fs::remove_dir_all(&dir);
}
