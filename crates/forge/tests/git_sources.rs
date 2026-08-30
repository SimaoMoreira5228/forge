mod common;

use common::*;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
	let output = std::process::Command::new("git")
		.args(args)
		.current_dir(dir)
		.env("GIT_AUTHOR_NAME", "forge")
		.env("GIT_AUTHOR_EMAIL", "forge@example.com")
		.env("GIT_COMMITTER_NAME", "forge")
		.env("GIT_COMMITTER_EMAIL", "forge@example.com")
		.output()
		.expect("run git");
	assert!(
		output.status.success(),
		"git {args:?}: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn make_repo(dir: &std::path::Path, body: &str) -> String {
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("Cargo.toml"),
		"[package]\nname = \"dep\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("src/lib.rs"),
		format!("pub fn hello() -> &'static str {{ \"{body}\" }}\n"),
	)
	.unwrap();
	git(dir, &["init", "--quiet", "-b", "main"]);
	git(dir, &["add", "-A"]);
	git(dir, &["commit", "--quiet", "-m", "init"]);
	git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn git_dependency_is_fetched_and_patch_git_redirects_it() {
	if !have_rustc() {
		eprintln!("skipping: no rustc");
		return;
	}
	if std::process::Command::new("git").arg("--version").output().is_err() {
		eprintln!("skipping: no git");
		return;
	}
	let root = std::env::temp_dir().join(format!("forge-igit-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	let repo_a = root.join("a");
	let repo_b = root.join("b");
	let rev_a = make_repo(&repo_a, "hello from git");
	let rev_b = make_repo(&repo_b, "hello from patch");
	let url_a = format!("file://{}", repo_a.display());
	let url_b = format!("file://{}", repo_b.display());

	let ws = root.join("ws");
	std::fs::create_dir_all(ws.join("src")).unwrap();
	std::fs::write(
		ws.join("FORGE_ROOT"),
		"[project]\nname = \"git_itest\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.rust]\nfrom = \"version\"\nversion = \"1.98.0\"\n",
	)
	.unwrap();
	std::fs::write(
		ws.join("FORGE.toml"),
		format!(
			"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.dep]\nversion = \"1.0.0\"\ngit = \"{url_a}\"\nrev = \"{rev_a}\"\n"
		),
	)
	.unwrap();
	std::fs::write(ws.join("src/main.rs"), "fn main() { println!(\"{}\", dep::hello()); }\n").unwrap();
	if !link_rust_toolchain(&ws) {
		eprintln!("skipping: Rust 1.98.0 toolchain is not installed");
		return;
	}

	let (ok, log) = run_forge(&ws, &["build"]);
	assert!(ok, "git dependency build failed: {log}");
	let binary = ws.join("forge-out/bin/debug/app");
	let output = std::process::Command::new(&binary).output().expect("run app");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello from git");

	let mut config = std::fs::read_to_string(ws.join("FORGE_ROOT")).unwrap();
	config.push_str(&format!("\n[patch.git.\"{url_a}\"]\ngit = \"{url_b}\"\nrev = \"{rev_b}\"\n"));
	std::fs::write(ws.join("FORGE_ROOT"), config).unwrap();

	let (ok, log) = run_forge(&ws, &["build"]);
	assert!(ok, "patched git build failed: {log}");
	let output = std::process::Command::new(&binary).output().expect("run app");
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello from patch");

	let _ = std::fs::remove_dir_all(&root);
}
