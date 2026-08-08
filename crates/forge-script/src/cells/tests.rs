use super::*;

fn hooks() -> CellHooks {
	CellHooks {
		obj_path: Box::new(|src| format!("forge-out/obj/{src}.o")),
		prior_depfile_headers: Box::new(|_| vec![]),
		lib_path: Box::new(|name| format!("forge-out/lib/lib{name}.a")),
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
                outputs: [ctx.obj_path(ctx.srcs[0])],
            });
        }
    "#;

#[test]
fn platform_matches_binding() {
	let mut engine = rhai::Engine::new();
	register_platform_matches(&mut engine);
	let mut profile = Map::new();
	profile.insert("is_debug".into(), Dynamic::from(true));
	let mut ctx = Map::new();
	ctx.insert("platform_os".into(), Dynamic::from("linux".to_string()));
	ctx.insert("platform_arch".into(), Dynamic::from("x86_64".to_string()));
	ctx.insert("platform_abi".into(), Dynamic::from("gnu".to_string()));
	ctx.insert("profile".into(), Dynamic::from(profile));
	let mut scope = rhai::Scope::new();
	scope.push("ctx", ctx);
	assert!(
		!engine
			.eval_with_scope::<bool>(&mut scope, "ctx.platform_matches(\"cfg(windows)\")")
			.unwrap()
	);
	assert!(
		engine
			.eval_with_scope::<bool>(&mut scope, "ctx.platform_matches(\"x86_64-unknown-linux-gnu\")")
			.unwrap()
	);
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
	scope.push("ctx", Map::new());
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
fn replace_in_action() {
	let script = r#"
            fn build(ctx) {
                let s = "hello.a";
			s.replace(".a", ".rlib");
			let outputs = [s];
                ctx.action(#{
                    name: "test",
                    command: "echo",
                    args: ["hello"],
                    inputs: [],
                    outputs: outputs,
                    env: #{},
                    toolchain_id: "",
                });
            }
        "#;
	let actions = lower(script, &component(), hooks()).unwrap();
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].outputs, vec![("hello.rlib".to_string(), false)]);
}
