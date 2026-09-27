mod forge_cli;
mod rust_toolchain;

use std::path::{Path, PathBuf};

use forge_cli::{action_keys, run_forge_in};

fn rust_workspace(root: &Path) -> PathBuf {
	std::fs::create_dir_all(root.join("src")).unwrap();
	std::fs::create_dir_all(root.join("vendor/helper/src")).unwrap();
	std::fs::write(
		root.join("FORGE_ROOT"),
		"[project]\nname = \"keysite\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.rust]\nfrom = \"version\"\nversion = \"1.98.0\"\n\n[patch.local.helper]\npath = \"vendor/helper\"\n",
	)
	.unwrap();
	std::fs::write(
		root.join("FORGE.toml"),
		"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust]\nbuild = true\n\n[binary.app.metadata.rust.build-dependencies.helper]\nversion = \"1.0.0\"\nsource = \"https://example.invalid/helper.tar.gz\"\nchecksum = \"fixture\"\n",
	)
	.unwrap();
	std::fs::write(
		root.join("forge.lock"),
		"version = 1\n\n[[packages]]\nname = \"helper\"\nversion = \"1.0.0\"\nsource = \"https://example.invalid/helper.tar.gz\"\nchecksum = \"fixture\"\n",
	)
	.unwrap();
	std::fs::write(
		root.join("vendor/helper/Cargo.toml"),
		"[package]\nname = \"helper\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
	)
	.unwrap();
	std::fs::write(root.join("vendor/helper/src/lib.rs"), "pub fn value() -> u32 { 42 }\n").unwrap();
	std::fs::write(
		root.join("build.rs"),
		"fn main() {\n    let generated = std::path::Path::new(&std::env::var(\"OUT_DIR\").unwrap()).join(\"gen.rs\");\n    std::fs::write(generated, format!(\"pub const N: u32 = {};\\n\", helper::value())).unwrap();\n}\n",
	)
	.unwrap();
	std::fs::write(
		root.join("src/main.rs"),
		"include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));\nfn main() { println!(\"{N}\"); }\n",
	)
	.unwrap();
	root.to_path_buf()
}

fn build_with_store(workspace: &Path, store: &Path) -> String {
	let (ok, log) = run_forge_in(workspace, &["build"], store);
	assert!(ok, "build failed in {}: {log}", workspace.display());
	log
}

#[test]
fn action_keys_do_not_depend_on_where_the_store_or_the_workspace_lives() {
	if !rust_toolchain::have_rustc() {
		eprintln!("skipping: no rustc");
		return;
	}
	let root = std::env::temp_dir().join(format!("forge-keysite-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);

	let mut stores = Vec::new();
	let mut workspaces = Vec::new();
	for name in ["one", "two"] {
		let workspace = rust_workspace(&root.join(name));
		let store = root.join(name).join("store");
		link_store_toolchains(&store);
		let (ok, log) = run_forge_in(&workspace, &["deps", "lock"], &store);
		assert!(ok, "deps lock failed: {log}");
		stores.push(store);
		workspaces.push(workspace);
	}

	for (workspace, store) in workspaces.iter().zip(&stores) {
		build_with_store(workspace, store);
	}

	let first = action_keys(&workspaces[0]);
	let second = action_keys(&workspaces[1]);
	assert!(!first.is_empty(), "the fixture must plan at least one action");
	assert_eq!(first, second, "a key may not depend on the store or workspace location");

	let _ = std::fs::remove_dir_all(&root);
}

fn link_store_toolchains(store: &Path) {
	std::fs::create_dir_all(store).unwrap();
	let source = forge_engine::store::store_root().join("toolchains");
	if !source.join("rust/1.98.0").exists() {
		panic!("the Rust 1.98.0 toolchain must be synced for this test");
	}
	#[cfg(unix)]
	std::os::unix::fs::symlink(&source, store.join("toolchains")).unwrap();
	#[cfg(not(unix))]
	std::fs::create_dir_all(store.join("toolchains/rust")).unwrap();
	#[cfg(not(unix))]
	std::fs::copy(
		forge_engine::store::store_root().join("toolchains/rust/1.98.0"),
		store.join("toolchains/rust/1.98.0"),
	)
	.unwrap();
}
