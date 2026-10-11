use super::*;

fn hooks() -> CellHooks {
	CellHooks {
		workspace: WorkspaceHooks {
			read_file: Box::new(|_| Err("not implemented in tests".into())),
			glob: Box::new(|_| Ok(vec![])),
		},
		artifact_path: Box::new(|src, category| Ok(format!("forge-out/{category}/{src}"))),
		depfile_inputs: Box::new(|_| Ok(vec![])),
		lib_path: Box::new(|filename| format!("forge-out/lib/modules.a/{filename}")),
		bin: Box::new(|name| format!("/tools/bin/{name}")),
		tool_bin: Box::new(|name| format!("/tools/bin/{name}")),
		toolchain_dir: Box::new(|name| format!("/tools/{name}")),
		toolchain_glob: Box::new(|_, _| Ok(vec![])),
		toolchain_read: Box::new(|_, _| Err("not implemented in tests".into())),
		tool_id: Box::new(|| "gcc@abc123".into()),
	}
}

fn session() -> CellSession {
	CellSession {
		workspace: "/workspace".into(),
		profile: ProfileView {
			name: "debug".into(),
			opt_level: "0".into(),
			debug: "full".into(),
			..Default::default()
		},
		..Default::default()
	}
}

#[test]
fn go_cell_compiles_and_links_a_binary_without_imports() {
	let script = crate::std_cells::cell_script("go").expect("go cell is embedded");
	let mut view = component();
	view.label = "//:app".into();
	view.name = "app".into();
	view.kind = "binary".into();
	view.srcs = vec!["src/main.go".into()];
	view.hdrs = vec![];
	let mut session = session();
	session.platform_os = "linux".into();
	session.platform_arch = "x86_64".into();
	let planned = plan(script, &session, hooks().workspace).unwrap();
	view.session = session.clone();
	let hooks = CellHooks {
		workspace: WorkspaceHooks {
			read_file: Box::new(|path| {
				if path == "src/main.go" {
					Ok("package main\n\nfunc main() {}\n".into())
				} else {
					Err(format!("missing {path}"))
				}
			}),
			glob: Box::new(|_| Ok(vec![])),
		},
		artifact_path: Box::new(|src, category| Ok(format!("forge-out/{category}/{src}"))),
		depfile_inputs: Box::new(|_| Ok(vec![])),
		lib_path: Box::new(|filename| format!("forge-out/lib/debug/app/{filename}")),
		bin: Box::new(|name| format!("/tools/bin/{name}")),
		tool_bin: Box::new(|_| String::new()),
		toolchain_dir: Box::new(|name| format!("/tools/{name}")),
		toolchain_glob: Box::new(|_, pattern| {
			if pattern.contains("runtime") {
				Ok(vec!["go/src/runtime/a.go".to_string()])
			} else {
				Ok(vec![])
			}
		}),
		toolchain_read: Box::new(|_, _| Err("not implemented in tests".into())),
		tool_id: Box::new(|| "go@abc123".into()),
	};
	let actions = lower(script, &planned, &view, hooks).unwrap();
	assert_eq!(actions.len(), 2);
	assert_eq!(actions[0].name, "go compile main");
	assert_eq!(actions[0].command, "/tools/bin/go");
	assert!(actions[0].args.contains(&"-p".to_string()));
	assert!(actions[0].args.contains(&"main".to_string()));
	assert_eq!(actions[1].name, "go link forge-out/bin/debug/app");
	assert_eq!(actions[1].command, "/tools/bin/go");
}

#[test]
fn go_cell_builds_stdlib_closure_with_importcfgs() {
	let script = crate::std_cells::cell_script("go").expect("go cell is embedded");
	let mut view = component();
	view.label = "//:greet".into();
	view.name = "greet".into();
	view.kind = "library".into();
	view.srcs = vec!["lib/greet.go".into()];
	view.hdrs = vec![];
	let mut session = session();
	session.platform_os = "linux".into();
	session.platform_arch = "x86_64".into();
	let planned = plan(script, &session, hooks().workspace).unwrap();
	view.session = session.clone();
	let std_source = |path: &str| match path {
		"go/src/fmt/fmt.go" => Ok("package fmt\n\nimport \"errors\"\n".to_string()),
		"go/src/errors/errors.go" => Ok("package errors\n".to_string()),
		_ => Err(format!("missing {path}")),
	};
	let std_hits = |pattern: &str| match pattern {
		"go/src/runtime/*.go" => Ok(vec!["go/src/runtime/a.go".to_string()]),
		"go/src/fmt/*.go" => Ok(vec!["go/src/fmt/fmt.go".to_string()]),
		"go/src/errors/*.go" => Ok(vec!["go/src/errors/errors.go".to_string()]),
		_ => Ok(vec![]),
	};
	let hooks = CellHooks {
		workspace: WorkspaceHooks {
			read_file: Box::new(|path| {
				if path == "lib/greet.go" {
					Ok("package greet\n\nimport \"fmt\"\n".into())
				} else {
					Err(format!("missing {path}"))
				}
			}),
			glob: Box::new(|_| Ok(vec![])),
		},
		artifact_path: Box::new(|src, category| Ok(format!("forge-out/{category}/{src}"))),
		depfile_inputs: Box::new(|_| Ok(vec![])),
		lib_path: Box::new(|filename| format!("forge-out/lib/debug/greet/{filename}")),
		bin: Box::new(|name| format!("/tools/bin/{name}")),
		tool_bin: Box::new(|_| String::new()),
		toolchain_dir: Box::new(|name| format!("/tools/{name}")),
		toolchain_glob: Box::new(move |_, pattern| std_hits(pattern)),
		toolchain_read: Box::new(move |_, path| std_source(path)),
		tool_id: Box::new(|| "go@abc123".into()),
	};
	let actions = lower(script, &planned, &view, hooks).unwrap();
	let names: Vec<&str> = actions.iter().map(|action| action.name.as_str()).collect();
	assert!(names.contains(&"go compile fmt"), "{names:?}");
	assert!(names.contains(&"go compile errors"), "{names:?}");
	assert!(names.contains(&"go compile greet"), "{names:?}");
	let lib = actions.iter().find(|action| action.name == "go compile greet").unwrap();
	assert_eq!(lib.outputs, vec![("forge-out/lib/debug/greet/libgreet.a".to_string(), false)]);
	assert_eq!(lib.artifact.as_deref(), Some("forge-out/lib/debug/greet/libgreet.a"));
	let fmt = actions.iter().find(|action| action.name == "go compile fmt").unwrap();
	let fmt_cfg = fmt.writes.values().next().expect("importcfg write");
	assert!(fmt_cfg.contains("packagefile errors="), "{fmt_cfg}");
}

#[test]
fn go_cell_builds_external_test_with_generated_main() {
	let script = crate::std_cells::cell_script("go").expect("go cell is embedded");
	let mut view = component();
	view.label = "//:greet_test".into();
	view.name = "greet_test".into();
	view.kind = "test".into();
	view.srcs = vec!["lib/greet_test.go".into()];
	view.hdrs = vec![];
	view.dep_artifacts = vec![("greet".into(), "forge-out/lib/debug/libgreet.a".into())];
	let mut session = session();
	session.platform_os = "linux".into();
	session.platform_arch = "x86_64".into();
	let mut packages = toml::Table::new();
	packages.insert("greet".into(), toml::Value::String("example.com/go-native/greet".into()));
	let mut cell_config = toml::Table::new();
	cell_config.insert("packages".into(), toml::Value::Table(packages));
	session.cell_config = cell_config;
	let planned = plan(script, &session, hooks().workspace).unwrap();
	view.session = session.clone();
	let hooks = CellHooks {
		workspace: WorkspaceHooks {
			read_file: Box::new(|path| {
				if path == "lib/greet_test.go" {
					Ok("package greet_test\n\nimport \"example.com/go-native/greet\"\n\nfunc TestAnswer() {}\n".into())
				} else {
					Err(format!("missing {path}"))
				}
			}),
			glob: Box::new(|_| Ok(vec![])),
		},
		artifact_path: Box::new(|src, category| Ok(format!("forge-out/{category}/{src}"))),
		depfile_inputs: Box::new(|_| Ok(vec![])),
		lib_path: Box::new(|filename| format!("forge-out/lib/debug/{filename}")),
		bin: Box::new(|name| format!("/tools/bin/{name}")),
		tool_bin: Box::new(|_| String::new()),
		toolchain_dir: Box::new(|name| format!("/tools/{name}")),
		toolchain_glob: Box::new(|_, pattern| {
			if pattern.contains("runtime") {
				Ok(vec!["go/src/runtime/a.go".to_string()])
			} else {
				Ok(vec![])
			}
		}),
		toolchain_read: Box::new(|_, _| Err("not implemented in tests".into())),
		tool_id: Box::new(|| "go@abc123".into()),
	};
	let actions = lower(script, &planned, &view, hooks).unwrap();
	let names: Vec<&str> = actions.iter().map(|action| action.name.as_str()).collect();
	assert!(names.contains(&"go compile main"), "{names:?}");
	assert!(names.contains(&"go compile example.com/go-native/greet_test"), "{names:?}");
	let main = actions.iter().find(|action| action.name == "go compile main").unwrap();
	assert!(main.inputs.is_empty(), "{:?}", main.inputs);
	assert!(
		main.writes.contains_key("forge-out/test/debug/greet_test.main.go"),
		"{:?}",
		main.writes.keys().collect::<Vec<_>>()
	);
	let main_src = &main.writes["forge-out/test/debug/greet_test.main.go"];
	assert!(main_src.contains("testimport0"), "{main_src}");
	assert!(main_src.contains("TestAnswer"), "{main_src}");
}

fn component() -> ComponentView {
	ComponentView {
		label: "//lib:math".into(),
		name: "math".into(),
		kind: "library".into(),
		srcs: vec!["lib/math.c".into()],
		hdrs: vec!["lib/math.h".into()],
		env: BTreeMap::new(),
		session: session(),
		..Default::default()
	}
}

fn plan_of(script: &str) -> CellPlan {
	plan(script, &session(), hooks().workspace).unwrap()
}

const WORKSPACE_ONLY_PLAN: &str = "fn plan(ctx) { () }";

fn plan_workspace() -> CellPlan {
	plan_of(WORKSPACE_ONLY_PLAN)
}

const MINIMAL_CELL: &str = r#"
        fn plan(ctx) { () }
        fn build(ctx, plan) {
            ctx.action(#{
                name: `compile ${ctx.srcs[0]}`,
                command: ctx.bin("cc"),
                args: ["-c", ctx.srcs[0]],
                inputs: ctx.srcs,
                outputs: [ctx.artifact_path(ctx.srcs[0], "obj") + ".o"],
            });
        }
    "#;

#[test]
fn graph_ops_binding() {
	let mut engine = rhai::Engine::new();
	register_graph_ops(&mut engine);
	let roots: rhai::Array = engine.eval("graph_roots(#{ a: [\"b\"], b: [] })").unwrap();
	let roots: Vec<String> = roots.into_iter().filter_map(|v| v.into_string().ok()).collect();
	assert_eq!(roots, vec!["a".to_string()]);
	let reachable: rhai::Array = engine
		.eval("graph_reachable(#{ a: [\"b\", \"c\"], b: [\"c\"], c: [] }, [\"a\"])")
		.unwrap();
	assert_eq!(reachable.len(), 3);
}
#[test]
fn every_lowering_is_handed_the_same_plan_handle() {
	let script = r#"
        fn plan(ctx) { #{ phase: "workspace" } }
        fn build(ctx, plan) {
            if plan.once("workspace owner") {
                ctx.action(#{ name: "plan owner", command: "true", outputs: ["forge-out/plan-owner"] });
            }
            if plan.get("phase") != "workspace" { throw "the lowering cannot read the plan it was handed"; }
            ctx.action(#{ name: `compile ${ctx.label}`, command: "true", outputs: [ctx.artifact_path(ctx.srcs[0], "obj")] });
        }
    "#;
	let planned = plan_of(script);
	let mut owners = 0;
	let mut compiled = 0;
	for index in 0..4 {
		let mut view = component();
		view.label = format!("//lib:math{index}");
		let actions = lower(script, &planned, &view, hooks()).unwrap();
		owners += actions.iter().filter(|action| action.name == "plan owner").count();
		compiled += actions.len();
	}
	assert_eq!(owners, 1, "a plan claim must reach the next lowering, not a fresh copy");
	assert_eq!(compiled, 4 + 1);
	assert_eq!(planned.section("phase").into_string().as_deref(), Ok("workspace"));
}

#[test]
fn the_rust_cell_owns_target_predicates() {
	let script = "fn plan(ctx) { () }\nfn build(ctx, plan) { ctx.platform_matches(\"cfg(unix)\"); }";
	let error = lower(script, &plan_of(script), &component(), hooks()).unwrap_err();
	assert!(error.to_string().contains("Function not found: platform_matches"), "{error}");
}

#[test]
fn cell_emits_actions_with_helpers() {
	let actions = lower(MINIMAL_CELL, &plan_of(MINIMAL_CELL), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].name, "compile lib/math.c");
	assert_eq!(actions[0].command, "/tools/bin/cc");
	assert_eq!(actions[0].outputs, vec![("forge-out/obj/lib/math.c.o".to_string(), false)]);
	assert_eq!(actions[0].toolchain_id, None);
}

#[test]
fn missing_required_field_is_an_error() {
	let bad = r#"
            fn plan(ctx) { () }
            fn build(ctx, plan) {
                ctx.action(#{ command: "x" });
            }
        "#;
	assert!(lower(bad, &plan_of(bad), &component(), hooks()).is_err());
}

#[test]
fn throw_in_cell_is_reported() {
	let script = r#"
            fn plan(ctx) { () }
            fn build(ctx, plan) {
                throw `no toolchain`;
            }
        "#;
	let err = lower(script, &plan_of(script), &component(), hooks()).unwrap_err();
	assert!(format!("{err}").contains("no toolchain"));
}

#[test]
fn output_variable_in_action_map() {
	let script = r#"
            fn plan(ctx) { () }
            fn build(ctx, plan) {
                let out = "forge-out/bin/debug/app";
                let outputs_arr = [out];
                ctx.action(#{
                    name: "test",
                    command: "echo",
                    args: ["hello"],
                    inputs: [],
                    outputs: outputs_arr,
                    env: #{},
                    toolchain_id: "",
                });
            }
        "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].outputs, vec![("forge-out/bin/debug/app".to_string(), false)]);
}

#[test]
fn args_variable_in_action_map() {
	let script = r#"
            fn plan(ctx) { () }
            fn build(ctx, plan) {
                let args_list = ["--edition=2024", "-o", "foo.rlib"];
                ctx.action(#{
                    name: "test",
                    command: "rustc",
                    args: args_list,
                    inputs: [],
                    outputs: ["foo.rlib"],
                    env: #{},
                    toolchain_id: "",
                });
            }
        "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].args, vec!["--edition=2024", "-o", "foo.rlib"]);
}

#[test]
fn fn_param_as_args_in_action() {
	let script = r#"
            fn plan(ctx) { () }
            fn do_compile(ctx, args_list, out_path) {
                ctx.action(#{
                    name: "compile",
                    command: "rustc",
                    args: args_list,
                    inputs: [],
                    outputs: [out_path],
                    env: #{},
                    toolchain_id: "",
                });
            }
            fn build(ctx, plan) {
                do_compile(ctx, ["--edition=2024"], "foo.rlib");
            }
        "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].args, vec!["--edition=2024"]);
	assert_eq!(actions[0].outputs, vec![("foo.rlib".to_string(), false)]);
}

#[test]
fn rust_cell_pattern() {
	let script = r#"
            fn plan(ctx) { () }
            fn build(ctx, plan) {
                let rustc = "/usr/bin/rustc";
                let out_path = "forge-out/bin/debug/app";
                let args_list = ["--crate-type", "bin", "-o", out_path];
                ctx.action(#{
                    name: "rustc " + ctx.label,
                    command: rustc,
                    args: args_list,
                    inputs: ctx.srcs,
                    outputs: [`${out_path}`],
                    env: #{},
                    toolchain_id: "",
                });
            }
        "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].args, vec!["--crate-type", "bin", "-o", "forge-out/bin/debug/app"]);
	assert_eq!(actions[0].outputs, vec![("forge-out/bin/debug/app".to_string(), false)]);
}

#[test]
fn rust_cell_full_compile_pattern() {
	let script = r#"
            fn plan(ctx) { () }
            fn profile_args(ctx) {
                let args = [];
                if ctx.profile.opt_level == 3 {
                    args.push("-O");
                }
                args;
            }
            fn compile_crate(ctx, crate_type, out_path) {
                let args = profile_args(ctx);
                args.push("--crate-type");
                args.push(crate_type);
                args.push("--crate-name");
                args.push("foo");
                args.push("-o");
                args.push(out_path);
                args.push("main.rs");
                let inputs = ctx.srcs;
                ctx.action(#{
                    name: "compile",
                    command: "/usr/bin/rustc",
                    args: args,
                    inputs: inputs,
                    outputs: [out_path],
                    env: #{},
                    toolchain_id: "",
                });
            }
            fn build(ctx, plan) {
                compile_crate(ctx, "bin", "forge-out/bin/debug/app");
            }
        "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].outputs, vec![("forge-out/bin/debug/app".to_string(), false)]);
}

#[test]
fn prebuilt_map_works() {
	let script = r#"
            fn plan(ctx) { () }
            fn build(ctx, plan) {
                let args = ["--crate-type", "bin"];
                let spec = #{
                    name: "test",
                    command: "echo",
                    args: args,
                    inputs: [],
                    outputs: ["out.txt"],
                    env: #{},
                    toolchain_id: "",
                };
                ctx.action(spec);
            }
        "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].outputs, vec![("out.txt".to_string(), false)]);
}

#[test]
fn artifact_and_depfile_callbacks_receive_exact_arguments() {
	let mut hooks = hooks();
	hooks.artifact_path = Box::new(|source, category| {
		assert_eq!((source, category), ("lib/math.c", "custom"));
		Ok("forge-out/custom/stem".into())
	});
	hooks.depfile_inputs = Box::new(|path| {
		assert_eq!(path, "forge-out/custom/stem.dependencies");
		Ok(vec!["missing.unusual".into(), "extensionless".into()])
	});
	let cell = r#"
        fn plan(ctx) { () }
        fn build(ctx, plan) {
			let stem = ctx.artifact_path(ctx.srcs[0], "custom");
			ctx.action(#{ name: "generic", command: "tool", outputs: [stem + ".xyz"],
				inputs: ctx.depfile_inputs(stem + ".dependencies") });
		}
    "#;
	let actions = lower(cell, &plan_of(cell), &component(), hooks).unwrap();
	assert_eq!(actions[0].inputs, ["missing.unusual", "extensionless"]);
	assert_eq!(actions[0].outputs, [("forge-out/custom/stem.xyz".into(), false)]);
}

#[test]
fn artifact_and_depfile_callback_errors_reach_the_cell() {
	for method in ["artifact_path(ctx.srcs[0], \"obj\")", "depfile_inputs(\"explicit.d\")"] {
		let mut hooks = hooks();
		hooks.artifact_path = Box::new(|_, _| Err("invalid artifact fragment".into()));
		hooks.depfile_inputs = Box::new(|_| Err("cannot read depfile".into()));
		let cell = format!("fn plan(ctx) {{ () }}\nfn build(ctx, plan) {{ ctx.{method}; }}");
		let error = lower(&cell, &plan_workspace(), &component(), hooks).unwrap_err();
		assert!(
			error.to_string().contains(if method.starts_with("artifact") {
				"invalid artifact fragment"
			} else {
				"cannot read depfile"
			}),
			"{error}"
		);
	}
}

#[test]
fn c_cell_owns_coverage_directory_and_suffixes() {
	for coverage in [false, true] {
		let mut view = component();
		view.session.profile.coverage = coverage;
		view.compiler = "gcc".into();
		let mut hooks = hooks();
		hooks.artifact_path = Box::new(move |source, category| {
			assert_eq!(source, "lib/math.c");
			assert_eq!(category, if coverage { "profile" } else { "obj" });
			Ok(format!("forge-out/{category}/a.o/stem"))
		});
		let prefix = if coverage { "profile" } else { "obj" };
		hooks.depfile_inputs = Box::new(move |path| {
			assert_eq!(path, format!("forge-out/{prefix}/a.o/stem.o.d"));
			Ok(vec![])
		});
		let script = crate::std_cells::cell_script("c").unwrap();
		let actions = lower(script, &plan_of(script), &view, hooks).unwrap();
		let mut outputs = vec![
			format!("forge-out/{prefix}/a.o/stem.o"),
			format!("forge-out/{prefix}/a.o/stem.o.d"),
		];
		if coverage {
			outputs.push(format!("forge-out/{prefix}/a.o/stem.gcno"));
		}
		assert_eq!(
			actions[0].outputs,
			outputs.into_iter().map(|p| (p, false)).collect::<Vec<_>>()
		);
	}
}

#[test]
fn c_cell_scopes_module_and_coverage_outputs_by_profile() {
	let mut view = component();
	view.kind = "test".into();
	view.srcs = vec!["lib/core.cppm".into()];
	view.session.profile.name = "asan".into();
	view.session.profile.coverage = true;
	let module = "export module math.core;\nexport int add(int a, int b) { return a + b; }\n".to_string();
	let mut hooks = hooks();
	hooks.bin = Box::new(|name| {
		if name.contains("clang") {
			format!("/tools/bin/{name}")
		} else {
			String::new()
		}
	});
	hooks.workspace.read_file = Box::new(move |_| Ok(module.clone()));
	let profile = view.session.profile.name.clone();
	let name = view.name.clone();
	hooks.artifact_path = Box::new(move |source, category| Ok(format!("forge-out/{category}/{profile}/{name}_{source}")));
	let script = crate::std_cells::cell_script("c").unwrap();
	let actions = lower(script, &plan_of(script), &view, hooks).unwrap();
	let precompile = actions
		.iter()
		.find(|action| action.name.starts_with("precompile module"))
		.expect("module interface precompile");
	assert_eq!(precompile.outputs, [("forge-out/mod/asan/math.core.pcm".into(), false)]);
	let run = actions
		.iter()
		.find(|action| action.name.starts_with("run "))
		.expect("test run");
	assert_eq!(run.outputs, [("forge-out/profile/asan/math_coverage".to_string(), true)]);
	assert_eq!(
		run.env.get("LLVM_PROFILE_FILE").map(String::as_str),
		Some("forge-out/profile/asan/math_coverage/%m.profraw")
	);
}

#[test]
fn c_cell_picks_lto_aware_archiver() {
	for (compiler, expected) in [("gcc", "/tools/bin/gcc-ar"), ("clang", "/tools/bin/llvm-ar")] {
		let mut view = component();
		view.kind = "library".into();
		view.compiler = compiler.into();
		view.session.profile.lto = "fat".into();
		let script = crate::std_cells::cell_script("c").unwrap();
		let actions = lower(script, &plan_of(script), &view, hooks()).unwrap();
		let archive = actions.iter().find(|action| action.name.starts_with("archive ")).unwrap();
		assert_eq!(archive.command, expected, "{compiler}");
	}
	let mut view = component();
	view.kind = "library".into();
	view.compiler = "gcc".into();
	let script = crate::std_cells::cell_script("c").unwrap();
	let actions = lower(script, &plan_of(script), &view, hooks()).unwrap();
	let archive = actions.iter().find(|action| action.name.starts_with("archive ")).unwrap();
	assert_eq!(archive.command, "/tools/bin/ar");
}

#[test]
fn library_path_preserves_cell_filenames() {
	let script = r#"
        fn plan(ctx) { () }
        fn build(ctx, plan) {
            ctx.action(#{
                name: "library",
                command: ctx.bin("tool"),
                outputs: [ctx.lib_path("plain"), ctx.lib_path("math.lib"), ctx.lib_path("libmath.so.1")],
            });
        }
    "#;
	let actions = lower(script, &plan_of(script), &component(), hooks()).unwrap();
	assert_eq!(
		actions[0].outputs,
		["plain", "math.lib", "libmath.so.1"].map(|filename| (format!("forge-out/lib/modules.a/{filename}"), false))
	);
}

#[test]
fn library_path_rejects_paths() {
	for filename in ["", ".", "..", "../out", "/out", "sub/out", r"sub\out", "C:out", "bad\0name"] {
		let mut view = component();
		view.name = filename.into();
		let mut hooks = hooks();
		hooks.lib_path = Box::new(|_| panic!("invalid filename reached the engine hook"));
		let cell = "fn plan(ctx) { () }\nfn build(ctx, plan) { ctx.lib_path(ctx.name); }";
		let error = lower(cell, &plan_workspace(), &view, hooks).unwrap_err();
		assert!(error.to_string().contains("lib_path requires a filename"), "{error}");
	}
}

#[test]
fn embedded_cells_choose_library_names_without_rewriting_directories() {
	for (language, source, filename) in [("c", "lib/math.c", "libmath.a"), ("rust", "lib/math.rs", "libmath.rlib")] {
		let mut view = component();
		view.srcs = vec![source.into()];
		let script = crate::std_cells::cell_script(language).unwrap();
		let actions = lower(script, &plan_of(script), &view, hooks()).unwrap();
		assert_eq!(
			actions.last().unwrap().outputs,
			vec![(format!("forge-out/lib/modules.a/{filename}"), false)]
		);
	}
}
