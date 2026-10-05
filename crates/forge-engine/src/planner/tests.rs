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
fn library_paths_are_profile_distinct() {
	let root = lib_path("debug", "", "libmath.a");
	assert_eq!(root, Path::new("forge-out/lib/debug/libmath.a"));
	assert_eq!(
		lib_path("release", "modules", "libgreet.a"),
		Path::new("forge-out/lib/release/modules/libgreet.a")
	);
	assert_ne!(root, lib_path("release", "", "libmath.a"));
	for path in [root, lib_path("release", "modules", "libgreet.a")] {
		assert!(path.starts_with("forge-out/lib"));
	}
}

#[test]
fn depfile_paths_match_both_runner_roots_without_testing_existence() {
	let workspace = Path::new("/workspace");
	let toolchains = [PathBuf::from("/toolchains/slint/1.18.1")];
	for path in [
		"./include/../missing.strange",
		"/workspace/missing.strange",
		"FORGE_EXEC_ROOT/missing.strange",
		"/workspace/forge-out/exec/missing.strange",
		"/workspace/forge-out/sandbox/123456789abc/missing.strange",
	] {
		assert_eq!(
			depfile_input_path(workspace, Path::new(path), &toolchains).unwrap(),
			Some(PathBuf::from("missing.strange"))
		);
	}
	for path in [
		"../escape",
		"FORGE_EXEC_ROOT/../escape",
		"/outside/input",
		"/workspace-other/input",
		".",
	] {
		assert!(depfile_input_path(workspace, Path::new(path), &toolchains).is_err(), "{path}");
	}
	assert_eq!(
		depfile_input_path(workspace, Path::new("FORGE_EXEC_ROOT_suffix/file"), &toolchains).unwrap(),
		Some(PathBuf::from("FORGE_EXEC_ROOT_suffix/file"))
	);
	assert_eq!(
		depfile_input_path(
			workspace,
			Path::new("/toolchains/slint/1.18.1/Slint-cpp-1.18.1-Linux-x86_64/include/slint/slint.h"),
			&toolchains
		)
		.unwrap(),
		None,
		"toolchain headers are keyed by reference, not hashed as inputs"
	);
}

const COUNTING_CELL: &str = r#"
fn plan(ctx) { #{ phase: "workspace" } }
fn build(ctx, plan) {
    if plan.get("phase") != "workspace" { throw "the lowering was handed no plan"; }
    if plan.once("workspace owner") {
        ctx.action(#{ name: "plan owner", command: "true", outputs: ["forge-out/plan-owner"] });
    }
    ctx.action(#{
        name: `compile ${ctx.label}`,
        command: "true",
        inputs: ctx.srcs,
        outputs: [ctx.artifact_path(ctx.srcs[0], "obj")],
        artifact: ctx.artifact_path(ctx.srcs[0], "lib") + ".a",
    });
}
"#;

fn library_decl(name: &str) -> forge_script::TargetDecl {
	let mut builder = forge_script::document::FieldsBuilder::new(forge_script::document::TargetKind::Library, name).unwrap();
	builder.string_list("srcs", vec![format!("src/{name}.c")]).unwrap();
	builder.finish().unwrap()
}

fn plan_n_libraries(count: usize) -> ActionDag {
	let workspace = std::env::temp_dir().join(format!("forge-plan-once-{}", std::process::id()));
	std::fs::create_dir_all(&workspace).unwrap();
	let cell = workspace.join("cell.rhai");
	std::fs::write(&cell, COUNTING_CELL).unwrap();

	let mut graph = BuildGraph::new();
	let mut decls = BTreeMap::new();
	for index in 0..count {
		let name = format!("lib{index}");
		let label = forge_core::Label::new("app", &name);
		decls.insert(label.to_string(), library_decl(&name));
		graph
			.add_component(forge_core::Component {
				label,
				kind: forge_core::ComponentKind::Library {
					link: forge_core::graph::component::LinkType::Static,
				},
				visibility: forge_core::Visibility::Public,
				compatible_with: Vec::new(),
				sources: vec![PathBuf::from(format!("src/{name}.c"))],
				headers: Vec::new(),
				configuration: forge_core::ConfigTransition::Target,
			})
			.unwrap();
	}

	let cells = StdCells::load(&workspace, &BTreeMap::from([("c".to_string(), cell)])).unwrap();
	let toolchains = BTreeMap::from([(
		"gcc".to_string(),
		ResolvedToolchain {
			name: "gcc".into(),
			root: workspace.clone(),
			bin_dir: workspace.join("bin"),
			path_dirs: vec![workspace.join("bin")],
			digest: "0".repeat(64),
			coverage: None,
			worker: None,
		},
	)]);
	let profile = Profile::debug();
	let platform = Platform::host();
	let cell_config = BTreeMap::new();
	let dag = build_action_dag(&PlanContext {
		graph: &graph,
		decls: &decls,
		profile: &profile,
		platform: &platform,
		toolchains: &toolchains,
		cells: &cells,
		cell_config: &cell_config,
		workspace: Some(&workspace),
		fetched_sources: &[],
		progress: None,
	})
	.unwrap();
	std::fs::remove_dir_all(&workspace).unwrap();
	dag
}

#[test]
fn a_cell_plans_the_workspace_once_and_every_component_shares_that_plan() {
	let dag = plan_n_libraries(4);
	let owners: Vec<&str> = dag
		.specs
		.iter()
		.filter(|spec| spec.name == "plan owner")
		.map(|spec| spec.component.as_str())
		.collect();
	assert_eq!(
		owners.len(),
		1,
		"the plan must be computed once for the workspace, not once per component"
	);
	assert_eq!(dag.specs.len(), 5, "one owner action plus one compile per component");
}
