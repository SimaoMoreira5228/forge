mod common;

use common::*;

#[test]
fn replay_reproduces_a_build_and_reports_divergence() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-irpl-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "build failed: {log}");

	let (ok, log) = run_forge(&dir, &["replay", "forge-out/forge.proof"]);
	assert!(ok, "pristine replay must reproduce bit-for-bit: {log}");
	assert!(log.contains("replay reproduced"), "{log}");

	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b + 7; }\n",
	)
	.unwrap();
	let (ok, log) = run_forge(&dir, &["replay", "forge-out/forge.proof"]);
	assert!(!ok, "a changed input must diverge under replay: {log}");
	assert!(log.contains("replay diverged"), "{log}");

	let _ = std::fs::remove_dir_all(&dir);
}
