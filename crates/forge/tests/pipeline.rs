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

#[test]
fn early_cutoff_prunes_downstream_when_outputs_are_unchanged() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-ieco-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);
	std::fs::write(
        dir.join("FORGE_ROOT"),
        "[project]\nname = \"eco\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n\n[profile.release]\nopt_level = 3\n",
    )
    .unwrap();

	let (ok, log) = run_forge(&dir, &["build", "--profile", "release"]);
	assert!(ok, "first build failed: {log}");

	let mut math = std::fs::read_to_string(dir.join("lib/math.c")).unwrap();
	math.push_str("// non-semantic edit\n");
	std::fs::write(dir.join("lib/math.c"), math).unwrap();

	let (_, log2) = run_forge(&dir, &["build", "--profile", "release"]);
	assert!(
		log2.contains("1 executed") && log2.contains("3 cache hits"),
		"expected only the changed compile to run, got: {log2}"
	);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn platform_overlays_activate_for_the_host() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let host_os = std::env::consts::OS;
	let dir = std::env::temp_dir().join(format!("forge-iover-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	std::fs::write(
        dir.join("src/main.c"),
        "#include <stdio.h>\n#ifndef HOST_PLATFORM_MARKER\n#error \"overlay did not activate\"\n#endif\nint main(void) { printf(\"ok\\n\"); return 0; }\n",
    )
    .unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[binary.app]\nsrcs = [\"src/main.c\"]\n\n[binary.app.target.\"os={host_os}\"]\ndefines = [\"HOST_PLATFORM_MARKER\"]\n"
		),
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "matching overlay must activate: {log}");

	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.c\"]\n\n[binary.app.target.\"os=plan9\"]\ndefines = [\"HOST_PLATFORM_MARKER\"]\n",
	)
	.unwrap();
	let (ok2, log2) = run_forge(&dir, &["build"]);
	assert!(!ok2, "non-matching overlay must not activate: {log2}");
	assert!(log2.contains("overlay did not activate"));

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn explain_names_the_changed_input() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-iexp-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	assert!(run_forge(&dir, &["build"]).0);

	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a * b; }\n",
	)
	.unwrap();
	let (_, log) = run_forge(&dir, &["explain", "//:math"]);
	assert!(log.contains("compile lib/math.c"), "{log}");
	assert!(log.contains("STALE"), "{log}");
	assert!(log.contains("changed:  lib/math.c"), "{log}");

	assert!(run_forge(&dir, &["build"]).0);
	let (_, log2) = run_forge(&dir, &["explain", "//:math"]);
	assert!(log2.contains("up to date"), "{log2}");
	assert!(!log2.contains("STALE"), "{log2}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flaky_tests_are_detected_and_reported() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-iflk-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"flk\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[test.flip]\nsrcs = [\"src/flip.c\"]\ndata = [\"mode.txt\"]\ntimeout_secs = 30\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/flip.c"),
		"#include <stdio.h>\nint main(void) { FILE *f = fopen(\"mode.txt\", \"r\"); return fgetc(f) == 'p' ? 0 : 1; }\n",
	)
	.unwrap();

	std::fs::write(dir.join("mode.txt"), "p").unwrap();
	let (ok1, log1) = run_forge(&dir, &["test"]);
	assert!(ok1, "first run must pass: {log1}");
	std::fs::write(dir.join("mode.txt"), "f").unwrap();
	let (ok2, log2) = run_forge(&dir, &["test"]);
	assert!(!ok2, "mode=f must fail: {log2}");
	std::fs::write(dir.join("mode.txt"), "p").unwrap();

	let (_, report) = run_forge(&dir, &["test", "--flake-report"]);
	assert!(report.contains("//:flip"), "{report}");
	assert!(report.contains("FLAKY"), "one pass + one fail is flaky: {report}");
	assert!(report.contains("50.0%"), "{report}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_normalizes_and_is_idempotent() {
	let dir = std::env::temp_dir().join(format!("forge-ifmt-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"fmt\"\n\n[discovery]\ninclude = [\".\"]\n",
	)
	.unwrap();
	std::fs::write(
        dir.join("FORGE.toml"),
        "[binary.app]\nsrcs    =   [\"main.c\"]\ndeps=[\"x\"]\n\n[binary.app.target.\"os=linux\"]\ndefines     =      [\"LINUX\"]\n",
    )
    .unwrap();

	let (ok, log) = run_forge(&dir, &["fmt"]);
	assert!(ok, "fmt failed: {log}");
	assert!(log.contains("reformatted"), "{log}");

	let formatted = std::fs::read_to_string(dir.join("FORGE.toml")).unwrap();
	assert!(formatted.contains("srcs = [\"main.c\"]"), "{formatted}");
	assert!(formatted.contains("deps = [\"x\"]"), "{formatted}");

	let (_, log2) = run_forge(&dir, &["fmt"]);
	assert!(log2.contains("all FORGE.toml files formatted"), "{log2}");

	std::fs::write(dir.join("FORGE.toml"), "[binary.app]\nsrcs     =      [\"main.c\"]\n").unwrap();
	let (ok3, log3) = run_forge(&dir, &["fmt", "--check"]);
	assert!(!ok3, "check must fail on unformatted input: {log3}");
	assert!(log3.contains("would reformat"), "{log3}");
	run_forge(&dir, &["fmt"]);
	let (ok4, log4) = run_forge(&dir, &["fmt", "--check"]);
	assert!(ok4, "check must pass after fmt: {log4}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn all_load_errors_are_reported_together() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-ibatch-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("app")).unwrap();
	std::fs::create_dir_all(dir.join("broken")).unwrap();

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"batch\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(dir.join("broken/FORGE.toml"), "[binary.bad]\nsrcc = [\"x.c\"]\n").unwrap();
	std::fs::write(
		dir.join("app/FORGE.toml"),
		"[binary.main]\ndeps = [\"//nope:missing\"]\nsrcs = []\n",
	)
	.unwrap();

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(!ok, "both errors must fail the build");
	assert!(log.contains("unknown key `srcc`"), "missing E102: {log}");
	assert!(
		log.contains("unknown dependency") || log.contains("unknown target"),
		"missing E002: {log}"
	);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gc_evicts_actions_and_forces_rebuild() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-igc-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	std::fs::write(
        dir.join("FORGE_ROOT"),
        "[project]\nname = \"gc\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n\n[build]\nmax_cache_bytes = 1\n",
    )
    .unwrap();

	let (_, first) = run_forge(&dir, &["build"]);
	assert!(first.contains("4 executed"), "{first}");
	let (_, second) = run_forge(&dir, &["build"]);
	assert!(second.contains("4 executed, 0 cache hits"), "{second}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn junit_report_reflects_pass_and_fail() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-ijunit-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"jr\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(dir.join("mode.txt"), "p").unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[test.good]\nsrcs = [\"src/good.c\"]\n\n[test.bad]\nsrcs = [\"src/bad.c\"]\ndata = [\"mode.txt\"]\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/good.c"), "int main(void){return 0;}\n").unwrap();
	std::fs::write(
		dir.join("src/bad.c"),
		"#include <stdio.h>\nint main(void){FILE*f=fopen(\"mode.txt\",\"r\");return fgetc(f)=='p'?0:1;}\n",
	)
	.unwrap();

	std::fs::write(dir.join("mode.txt"), "f").unwrap();
	let (_, _) = run_forge(&dir, &["test", "--output", "junit:report.xml"]);
	let report = std::fs::read_to_string(dir.join("report.xml")).unwrap();
	assert!(report.contains("<testsuites>"), "{report}");
	assert!(report.contains("<failure"), "bad test failed and must appear: {report}");
	assert!(
		report.contains("tests=\"1\" failures=\"1\"") || report.contains("failures=\"1\""),
		"{report}"
	);

	std::fs::write(dir.join("mode.txt"), "p").unwrap();
	assert!(run_forge(&dir, &["test"]).0);
	let (_, _) = run_forge(&dir, &["test", "--output", "junit:report.xml"]);
	let report2 = std::fs::read_to_string(dir.join("report.xml")).unwrap();
	assert!(!report2.contains("<failure"), "all green, but failure present: {report2}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_cache_and_test_flags_work() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-iclean-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();

	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"cl\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.c\"]\n\n[test.ok]\nsrcs = [\"src/main.c\"]\n",
	)
	.unwrap();
	std::fs::write(dir.join("src/main.c"), "int main(void){return 0;}\n").unwrap();

	assert!(run_forge(&dir, &["build"]).0);

	let cas = dir.join("forge-out/cas");
	assert!(cas.exists(), "cas must exist after a build");

	assert!(run_forge(&dir, &["clean", "--cache"]).0);
	assert!(!cas.exists(), "cas removed");

	assert!(run_forge(&dir, &["build"]).0);
	assert!(run_forge(&dir, &["test"]).0);
	let (ok1, log1) = run_forge(&dir, &["test"]);
	assert!(ok1, "{log1}");
	assert!(log1.contains("served from cache"), "second test run should be cached: {log1}");

	assert!(run_forge(&dir, &["clean", "--test"]).0);
	let second_test = run_forge(&dir, &["test"]);
	assert!(
		second_test.1.contains("1 executed, 0 served"),
		"verdict cleared: test re-ran; compilation stayed cached: {}",
		second_test.1
	);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stats_surface_telemetry_and_graph() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-istats-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	assert!(run_forge(&dir, &["build"]).0);
	let (ok, log) = run_forge(&dir, &["stats"]);
	assert!(ok, "{log}");

	assert!(log.contains("slowest actions:"), "{log}");
	assert!(log.contains("compile lib/math.c"), "action names must appear: {log}");
	assert!(log.contains("cache hit rates:"), "{log}");

	let conn = rusqlite::Connection::open(dir.join("forge-out/cas/cache.db")).unwrap();
	let components: Vec<(String, String)> = conn
		.prepare("SELECT label, kind FROM components ORDER BY label")
		.unwrap()
		.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
		.unwrap()
		.flatten()
		.collect();
	assert!(
		components.iter().any(|(l, k)| l == "//:math" && k == "library"),
		"{components:?}"
	);
	assert!(
		components.iter().any(|(l, k)| l == "//:app" && k == "binary"),
		"{components:?}"
	);
	let edges: i64 = conn
		.query_row("SELECT COUNT(*) FROM component_edges", [], |row| row.get(0))
		.unwrap();
	assert_eq!(edges, 1, "app -> math");

	assert!(run_forge(&dir, &["build"]).0);
	let max_ms: i64 = conn
		.query_row("SELECT MAX(duration_ms) FROM actions WHERE duration_ms > 0", [], |row| {
			row.get(0)
		})
		.unwrap_or(0);
	assert!(max_ms > 0, "hit path must preserve measured durations");

	let _ = std::fs::remove_dir_all(&dir);
}
