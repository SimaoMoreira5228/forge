mod common;

use common::*;

#[test]
fn proof_records_and_verifies_a_build() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-iproof-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "build failed: {log}");
	assert!(dir.join("forge-out/forge.proof").is_file(), "build must emit a proof");

	let (ok, log) = run_forge(&dir, &["verify"]);
	assert!(ok, "pristine proof must verify: {log}");
	assert!(log.contains("proof verified"), "{log}");

	std::fs::write(
		dir.join("lib/math.c"),
		"#include \"math.h\"\nint add(int a, int b) { return a + b + 1; }\n",
	)
	.unwrap();
	let (ok, log) = run_forge(&dir, &["verify"]);
	assert!(!ok, "a changed input must fail verification: {log}");
	assert!(log.contains("divergences"), "{log}");

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn time_travel_restores_outputs_from_a_proof() {
	if !have_compiler() {
		eprintln!("skipping: no system compiler");
		return;
	}
	let dir = std::env::temp_dir().join(format!("forge-itt-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	write_workspace(&dir);

	let (ok, log) = run_forge(&dir, &["build"]);
	assert!(ok, "build failed: {log}");
	let binary = dir.join("forge-out/bin/debug/app");
	assert!(binary.is_file());
	std::fs::remove_file(&binary).unwrap();

	let (ok, log) = run_forge(&dir, &["time-travel", "--proof", "forge-out/forge.proof"]);
	assert!(ok, "time-travel failed: {log}");
	assert!(binary.is_file(), "time-travel must restore the removed output: {log}");
	let output = std::process::Command::new(&binary).output().expect("run restored binary");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

	let _ = std::fs::remove_dir_all(&dir);
}
