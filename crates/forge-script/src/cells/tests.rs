use super::*;

fn hooks() -> CellHooks {
	CellHooks {
		artifact_path: Box::new(|src, category| Ok(format!("forge-out/{category}/{src}"))),
		depfile_inputs: Box::new(|_| Ok(vec![])),
		lib_path: Box::new(|filename| format!("forge-out/lib/modules.a/{filename}")),
		bin: Box::new(|name| format!("/tools/bin/{name}")),
		tool_id: Box::new(|| "gcc@abc123".into()),
		read_file: Box::new(|_| Err("not implemented in tests".into())),
		glob: Box::new(|_| Ok(vec![])),
	}
}

fn component() -> ComponentView {
	ComponentView {
		label: "//lib:math".into(),
		name: "math".into(),
		kind: "library".into(),
		srcs: vec!["lib/math.c".into()],
		hdrs: vec!["lib/math.h".into()],
		env: BTreeMap::new(),
		workspace: "/workspace".into(),
		profile: ProfileView {
			name: "debug".into(),
			opt_level: 0,
			debug: true,
			..Default::default()
		},
		..Default::default()
	}
}

const MINIMAL_CELL: &str = r#"
        fn build(ctx) {
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
fn rust_cell_owns_target_predicates() {
	let cell = crate::std_cells::cell_script("rust").unwrap();
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	let checks = r#"
let ctx = #{ platform_os: "linux", platform_arch: "x86_64", platform_abi: "gnu", profile: #{ is_debug: true } };
for sample in [
    ["cfg(windows)", false], ["cfg(unix)", true], ["cfg(debug_assertions)", true],
    ["cfg(target_os = \"linux\")", true], ["cfg(target_os = \"lin ux\")", false],
    ["cfg(target_arch = \"aarch64\")", false], ["cfg(target_pointer_width = \"64\")", true],
    ["cfg(all(any(target_os = \"linux\", target_os = \"android\"), not(any(all(target_os = \"linux\", target_env = \"\"), getrandom_backend = \"custom\"))))", true],
    ["cfg(all())", true], ["cfg(any())", false], ["cfg(all(unix,))", true],
    ["x86_64-unknown-linux-gnu", true], ["x86_64-pc-windows-msvc", false]
] {
    if rust_cfg_matches(ctx, sample[0]) != sample[1] { throw `incorrect cfg result: ${sample}`; }
}
for expression in ["cfg(not())", "cfg(not(unix, windows))", "cfg(all(unix)", "cfg(unix))", "cfg(all(,unix))", "cfg(target_os = linux)", "cfg(target_os = \"linux)", "cfg(unix windows)", "cfg(foo(unix))"] {
    let rejected = false;
    try { rust_cfg_matches(ctx, expression); } catch { rejected = true; }
    if !rejected { throw `accepted invalid cfg: ${expression}`; }
}
ctx.platform_os = "darwin";
if !rust_cfg_matches(ctx, "cfg(all(target_os = \"macos\", target_vendor = \"apple\"))") { throw "Darwin mapping failed"; }
ctx.platform_os = "windows";
if !rust_cfg_matches(ctx, "cfg(all(windows, not(unix), target_family = \"windows\"))") { throw "Windows mapping failed"; }
ctx.platform_os = "linux";
let manifest = #{ target: #{ "cfg(windows)": #{ dependencies: #{ win: "1" } }, "cfg(unix)": #{ dependencies: #{ posix: "1" } } } };
if active_dep_names(ctx, manifest) != ["posix"] { throw "target dependency filtering failed"; }
"#;
	engine.eval::<()>(&format!("{cell}\n{checks}")).unwrap();
	let error = lower(
		"fn build(ctx) { ctx.platform_matches(\"cfg(unix)\"); }",
		&component(),
		hooks(),
	)
	.unwrap_err();
	assert!(error.to_string().contains("Function not found: platform_matches"), "{error}");
}

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
fn feature_propagation_from_root() {
	let cell = crate::std_cells::cell_script("rust").expect("rust cell is embedded");
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	let ctx = Map::new();
	let mut scope = rhai::Scope::new();
	scope.push("ctx", ctx);
	let tail = r#"
let meta = #{
  "clap@4.6.7": #{
    src: #{ name: "clap", version: "4.6.7" },
    manifest: #{
      features: #{ "default": ["std"], "std": ["clap_builder/std"] },
      dependencies: #{ clap_builder: #{ "default-features": false } },
    },
  },
  "clap_builder@4.6.7": #{
    src: #{ name: "clap_builder", version: "4.6.7" },
    manifest: #{ features: #{ "std": [] }, dependencies: #{} },
  },
};
let adjacency = #{ "clap@4.6.7": ["clap_builder@4.6.7"], "clap_builder@4.6.7": [] };
let order = ["clap@4.6.7", "clap_builder@4.6.7"];
let requests = #{ "clap": #{ defaults: true, features: [] } };
let enabled = feature_sets(ctx, meta, adjacency, order, requests);
enabled["clap_builder@4.6.7"].contains("std")
"#;
	let program = format!("{cell}\n{tail}");
	let propagated = engine.eval_with_scope::<bool>(&mut scope, &program).unwrap();
	assert!(propagated, "clap_builder should inherit std via clap");
}

#[test]
fn workspace_root_requests_reads_members() {
	let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.unwrap()
		.parent()
		.unwrap()
		.to_path_buf();
	let cell = crate::std_cells::cell_script("rust").expect("rust cell is embedded");
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	let glob_root = repo.clone();
	engine.register_fn(
		"glob",
		move |_ctx: &mut Map, pattern: &str| -> Result<rhai::Array, Box<EvalAltResult>> {
			let hits =
				crate::glob::expand_glob(&glob_root, pattern).map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
			Ok(hits
				.into_iter()
				.map(|p| Dynamic::from(p.to_string_lossy().into_owned()))
				.collect())
		},
	);
	let read_root = repo.clone();
	engine.register_fn(
		"read_file",
		move |_ctx: &mut Map, path: &str| -> Result<String, Box<EvalAltResult>> {
			std::fs::read_to_string(read_root.join(path)).map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })
		},
	);
	engine.register_fn("toml_decode", |text: &str| -> Result<Map, Box<EvalAltResult>> {
		crate::rhai_rt::toml_decode(text).map_err(Into::into)
	});
	let mut scope = rhai::Scope::new();
	let mut ctx = Map::new();
	ctx.insert(
		"cell_config".into(),
		Dynamic::from(rhai::Map::from_iter([("mode".into(), Dynamic::from("cargo".to_string()))])),
	);
	scope.push("ctx", ctx);
	let tail = r#"
let requests = workspace_root_requests(ctx);
let has_clap = requests.contains("clap");
let defaults = if has_clap { requests["clap"]["defaults"] } else { false };
let has_derive = if has_clap { requests["clap"]["features"].contains("derive") } else { false };
#{ clap: has_clap, defaults: defaults, derive: has_derive }
"#;
	let program = format!("{cell}\n{tail}");
	let result: Map = engine.eval_with_scope(&mut scope, &program).unwrap();
	eprintln!("requests: {result:?}");
	assert_eq!(result.get("clap").and_then(|v| v.as_bool().ok()), Some(true));
	assert_eq!(result.get("derive").and_then(|v| v.as_bool().ok()), Some(true));
}

#[test]
fn cell_emits_actions_with_helpers() {
	let actions = lower(MINIMAL_CELL, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].name, "compile lib/math.c");
	assert_eq!(actions[0].command, "/tools/bin/cc");
	assert_eq!(actions[0].outputs, vec![("forge-out/obj/lib/math.c.o".to_string(), false)]);
	assert_eq!(actions[0].toolchain_id, None);
}

#[test]
fn missing_required_field_is_an_error() {
	let bad = r#"
            fn build(ctx) {
                ctx.action(#{ command: "x" });
            }
        "#;
	assert!(lower(bad, &component(), hooks()).is_err());
}

#[test]
fn throw_in_cell_is_reported() {
	let script = r#"
            fn build(ctx) {
                throw `no toolchain`;
            }
        "#;
	let err = lower(script, &component(), hooks()).unwrap_err();
	assert!(format!("{err}").contains("no toolchain"));
}

#[test]
fn output_variable_in_action_map() {
	let script = r#"
            fn build(ctx) {
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
	let actions = lower(script, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].outputs, vec![("forge-out/bin/debug/app".to_string(), false)]);
}

#[test]
fn args_variable_in_action_map() {
	let script = r#"
            fn build(ctx) {
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
	let actions = lower(script, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].args, vec!["--edition=2024", "-o", "foo.rlib"]);
}

#[test]
fn fn_param_as_args_in_action() {
	let script = r#"
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
            fn build(ctx) {
                do_compile(ctx, ["--edition=2024"], "foo.rlib");
            }
        "#;
	let actions = lower(script, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].args, vec!["--edition=2024"]);
	assert_eq!(actions[0].outputs, vec![("foo.rlib".to_string(), false)]);
}

#[test]
fn rust_cell_pattern() {
	let script = r#"
            fn build(ctx) {
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
	let actions = lower(script, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].args, vec!["--crate-type", "bin", "-o", "forge-out/bin/debug/app"]);
	assert_eq!(actions[0].outputs, vec![("forge-out/bin/debug/app".to_string(), false)]);
}

#[test]
fn rust_cell_full_compile_pattern() {
	let script = r#"
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
            fn build(ctx) {
                compile_crate(ctx, "bin", "forge-out/bin/debug/app");
            }
        "#;
	let actions = lower(script, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].outputs, vec![("forge-out/bin/debug/app".to_string(), false)]);
}

#[test]
fn prebuilt_map_works() {
	let script = r#"
            fn build(ctx) {
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
	let actions = lower(script, &component(), hooks()).unwrap();
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
	let actions = lower(
		r#"
		fn build(ctx) {
			let stem = ctx.artifact_path(ctx.srcs[0], "custom");
			ctx.action(#{ name: "generic", command: "tool", outputs: [stem + ".xyz"],
				inputs: ctx.depfile_inputs(stem + ".dependencies") });
		}
	"#,
		&component(),
		hooks,
	)
	.unwrap();
	assert_eq!(actions[0].inputs, ["missing.unusual", "extensionless"]);
	assert_eq!(actions[0].outputs, [("forge-out/custom/stem.xyz".into(), false)]);
}

#[test]
fn artifact_and_depfile_callback_errors_reach_the_cell() {
	for method in ["artifact_path(ctx.srcs[0], \"obj\")", "depfile_inputs(\"explicit.d\")"] {
		let mut hooks = hooks();
		hooks.artifact_path = Box::new(|_, _| Err("invalid artifact fragment".into()));
		hooks.depfile_inputs = Box::new(|_| Err("cannot read depfile".into()));
		let error = lower(&format!("fn build(ctx) {{ ctx.{method}; }}"), &component(), hooks).unwrap_err();
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
		view.profile.coverage = coverage;
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
		let actions = lower(crate::std_cells::cell_script("c").unwrap(), &view, hooks).unwrap();
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
fn library_path_preserves_cell_filenames() {
	let script = r#"
        fn build(ctx) {
            ctx.action(#{
                name: "library",
                command: ctx.bin("tool"),
                outputs: [ctx.lib_path("plain"), ctx.lib_path("math.lib"), ctx.lib_path("libmath.so.1")],
            });
        }
    "#;
	let actions = lower(script, &component(), hooks()).unwrap();
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
		let error = lower("fn build(ctx) { ctx.lib_path(ctx.name); }", &view, hooks).unwrap_err();
		assert!(error.to_string().contains("lib_path requires a filename"), "{error}");
	}
}

#[test]
fn embedded_cells_choose_library_names_without_rewriting_directories() {
	for (language, source, filename) in [("c", "lib/math.c", "libmath.a"), ("rust", "lib/math.rs", "libmath.rlib")] {
		let mut view = component();
		view.srcs = vec![source.into()];
		let script = crate::std_cells::cell_script(language).unwrap();
		let actions = lower(script, &view, hooks()).unwrap();
		assert_eq!(
			actions.last().unwrap().outputs,
			vec![(format!("forge-out/lib/modules.a/{filename}"), false)]
		);
	}
}
