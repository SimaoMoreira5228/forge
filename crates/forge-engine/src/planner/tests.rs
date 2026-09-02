use super::*;

#[test]
fn artifact_stems_are_generic_stable_and_namespaced() {
	let source = Path::new("src/math.c");
	let stem = artifact_path(source, "debug", "___math", "custom").unwrap();
	assert!(stem.starts_with("forge-out/custom/debug/___math_math_"));
	assert!(!stem.ends_with(".o"));
	assert_eq!(stem, artifact_path(source, "debug", "___math", "custom").unwrap());
	for (src, profile, namespace, category) in [
		("other/math.c", "debug", "___math", "custom"),
		("src/math.c", "release", "___math", "custom"),
		("src/math.c", "debug", "___other", "custom"),
		("src/math.c", "debug", "___math", "profile"),
	] {
		assert_ne!(stem, artifact_path(Path::new(src), profile, namespace, category).unwrap());
	}
	for fragment in ["", ".", "..", "../out", "/out", "sub/out", r"sub\out", "C:out", "bad\0name"] {
		assert!(artifact_path(source, "debug", "ns", fragment).is_err());
		assert!(artifact_path(source, fragment, "ns", "obj").is_err());
		assert!(artifact_path(source, "debug", fragment, "obj").is_err());
	}
	assert!(artifact_path(Path::new(r"bad\stem.c"), "debug", "ns", "obj").is_err());
}

#[test]
fn library_paths_are_profile_distinct_and_outside_the_dependency_registry() {
	let root = lib_path("debug", "", "libmath.rlib");
	assert_eq!(root, Path::new("forge-out/lib/debug/libmath.rlib"));
	assert_eq!(
		lib_path("release", "modules.a", "libgreet.a"),
		Path::new("forge-out/lib/release/modules.a/libgreet.a")
	);
	assert_ne!(root, lib_path("release", "", "libmath.rlib"));
	for path in [root, lib_path("release", "modules.a", "libgreet.a")] {
		assert!(path.starts_with("forge-out/lib"));
		assert!(!path.starts_with("forge-out/lib/deps"));
	}
}

#[test]
fn depfile_paths_match_both_runner_roots_without_testing_existence() {
	let workspace = Path::new("/workspace");
	for path in [
		"./include/../missing.strange",
		"/workspace/missing.strange",
		"FORGE_EXEC_ROOT/missing.strange",
		"/workspace/forge-out/exec/missing.strange",
		"/workspace/forge-out/sandbox/123456789abc/missing.strange",
	] {
		assert_eq!(
			depfile_input_path(workspace, Path::new(path)).unwrap(),
			Path::new("missing.strange")
		);
	}
	for path in [
		"../escape",
		"FORGE_EXEC_ROOT/../escape",
		"/outside/input",
		"/workspace-other/input",
		".",
	] {
		assert!(depfile_input_path(workspace, Path::new(path)).is_err(), "{path}");
	}
	assert_eq!(
		depfile_input_path(workspace, Path::new("FORGE_EXEC_ROOT_suffix/file")).unwrap(),
		Path::new("FORGE_EXEC_ROOT_suffix/file")
	);
}
