mod c_workspace;
mod forge_cli;

use c_workspace::have_compiler;
use forge_cli::run_forge;

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

#[test]
fn selected_binary_builds_generated_headers_and_static_archives() {
	if !have_compiler() {
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-generated-static-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	write_two_binaries(&dir);
	std::fs::write(
		dir.join("FORGE.toml"),
		r#"
[rule.headers]
command = "/bin/sh"
args = ["-c", "mkdir -p gen && printf 'int answer(void);\n' > gen/api.h"]
output_dir = "gen"

[rule.archive]
command = "/bin/sh"
args = ["-c", "gcc -c src/answer.c -o answer.o && ar rcs libanswer.a answer.o"]
inputs = ["src/answer.c"]
outputs = ["libanswer.a"]

[binary.app]
srcs = ["src/app.c"]
hdrs = ["gen/api.h"]
includes = ["gen"]
compiler = "gcc"
system_libs = ["m"]

[binary.app.metadata.c.static.answer]
lib = "libanswer.a"
"#,
	)
	.unwrap();
	std::fs::write(
		dir.join("src/answer.c"),
		"#include <math.h>\nint answer(void) { volatile double value = 1764; return (int)sqrt(value); }\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/app.c"),
		"#include \"api.h\"\nint main(void) { return answer() == 42 ? 0 : 1; }\n",
	)
	.unwrap();
	let (ok, log) = run_forge(&dir, &["build", "//:app"]);
	assert!(ok, "selected build failed: {log}");
	let binary = dir.join("forge-out/bin/debug/app");
	assert!(std::process::Command::new(binary).status().unwrap().success());
	let (ok, log) = run_forge(&dir, &["build", "//:app"]);
	assert!(
		ok && log.contains("0 executed"),
		"selected build did not reuse its cache: {log}"
	);
	std::fs::remove_dir_all(&dir).unwrap();
}
