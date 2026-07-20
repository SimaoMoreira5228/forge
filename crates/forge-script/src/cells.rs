use std::collections::BTreeMap;

use forge_diagnostics::{ForgeDiagnostic, codes};
use rhai::{Dynamic, EvalAltResult, Map};

#[derive(Debug, Clone)]
pub struct ActionDecl {
	pub name: String,
	pub command: String,
	pub args: Vec<String>,
	pub inputs: Vec<PathBufArg>,
	pub outputs: Vec<(PathBufArg, bool)>,
	pub env: BTreeMap<String, String>,
	pub toolchain_id: Option<String>,
}

type PathBufArg = String;

#[derive(Debug, Clone, Default)]
pub struct ComponentView {
	pub label: String,
	pub name: String,
	pub kind: String,
	pub compiler: String,
	pub srcs: Vec<String>,
	pub hdrs: Vec<String>,
	pub includes: Vec<String>,
	pub defines: Vec<String>,
	pub flags: Vec<String>,
	pub standard: Option<String>,
	pub system_libs: Vec<String>,
	pub run_args: Vec<String>,
	pub data: Vec<String>,
	pub env: BTreeMap<String, String>,
	pub dep_archives: Vec<String>,
	pub dep_artifacts: Vec<(String, String)>,
	pub workspace: String,
	pub linker: String,
	pub link_flags: Vec<String>,
	pub platform_os: String,
	pub platform_arch: String,
	pub platform_abi: String,
	pub profile: ProfileView,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileView {
	pub name: String,
	pub opt_level: i64,
	pub debug: bool,
	pub lto: bool,
	pub strip: bool,
	pub coverage: bool,
	pub defines: Vec<String>,
	pub sanitizers: Vec<String>,
}

type HeaderLookup = Box<dyn Fn(&str) -> Vec<String>>;

pub struct CellHooks {
	pub obj_path: Box<dyn Fn(&str) -> String>,
	pub prior_depfile_headers: HeaderLookup,
	pub lib_path: Box<dyn Fn(&str) -> String>,
	pub bin: Box<dyn Fn(&str) -> String>,
	pub tool_id: Box<dyn Fn() -> String>,
	pub read_file: FileReader,
}

type FileReader = Box<dyn Fn(&str) -> Result<String, String>>;

pub fn lower(script: &str, component: &ComponentView, hooks: CellHooks) -> Result<Vec<ActionDecl>, ForgeDiagnostic> {
	let actions: std::rc::Rc<std::cell::RefCell<Vec<ActionDecl>>> = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);

	register_emitters(&mut engine, &actions);
	register_helpers(&mut engine, component, hooks);

	let ctx = context_map(component);
	let mut scope = rhai::Scope::new();
	scope.push("ctx", ctx);

	let program = format!("{script}\nbuild(ctx);");
	engine
		.eval_with_scope::<()>(&mut scope, &program)
		.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("cell failed: {e}")))?;

	Ok(std::mem::take(&mut *actions.borrow_mut()))
}

fn register_emitters(engine: &mut rhai::Engine, actions: &std::rc::Rc<std::cell::RefCell<Vec<ActionDecl>>>) {
	let sink = std::rc::Rc::clone(actions);
	engine.register_fn("action", move |ctx: &mut Map, spec: Map| -> Result<(), Box<EvalAltResult>> {
		let _ = ctx;
		let decl = parse_action(spec)?;
		sink.borrow_mut().push(decl);
		Ok(())
	});
}

fn register_helpers(engine: &mut rhai::Engine, component: &ComponentView, hooks: CellHooks) {
	let CellHooks {
		obj_path,
		prior_depfile_headers,
		lib_path,
		bin,
		tool_id,
		read_file,
	} = hooks;

	engine.register_fn("bin", move |_ctx: &mut Map, name: &str| -> String { bin(name) });
	engine.register_fn("tool_id", move |_ctx: &mut Map| -> String { tool_id() });
	engine.register_fn("lib_path", move |_ctx: &mut Map, name: &str| -> String { lib_path(name) });
	engine.register_fn("obj_path", move |_ctx: &mut Map, src: &str| -> String { obj_path(src) });
	engine.register_fn("prior_depfile_headers", move |_ctx: &mut Map, obj: &str| -> rhai::Array {
		prior_depfile_headers(obj).into_iter().map(Dynamic::from).collect()
	});
	engine.register_fn(
		"read_file",
		move |_ctx: &mut Map, path: &str| -> Result<String, Box<EvalAltResult>> { read_file(path).map_err(|e| e.into()) },
	);
	let _ = component.profile;
}

fn context_map(component: &ComponentView) -> Map {
	let mut ctx = Map::new();
	insert_str(&mut ctx, "label", &component.label);
	insert_str(&mut ctx, "name", &component.name);
	insert_str(&mut ctx, "kind", &component.kind);
	insert_str(&mut ctx, "compiler", &component.compiler);
	insert_list(&mut ctx, "srcs", &component.srcs);
	insert_list(&mut ctx, "hdrs", &component.hdrs);
	insert_list(&mut ctx, "includes", &component.includes);
	insert_list(&mut ctx, "defines", &component.defines);
	insert_list(&mut ctx, "flags", &component.flags);
	insert_str(&mut ctx, "standard", component.standard.as_deref().unwrap_or(""));
	insert_list(&mut ctx, "system_libs", &component.system_libs);
	insert_list(&mut ctx, "run_args", &component.run_args);
	insert_list(&mut ctx, "data", &component.data);
	insert_list(&mut ctx, "dep_archives", &component.dep_archives);
	let artifact_list: rhai::Array = component
		.dep_artifacts
		.iter()
		.map(|(name, path)| Dynamic::from(vec![Dynamic::from(name.clone()), Dynamic::from(path.clone())]))
		.collect();
	ctx.insert("dep_artifacts".into(), Dynamic::from(artifact_list));
	insert_str(&mut ctx, "workspace", &component.workspace);
	insert_str(&mut ctx, "linker", &component.linker);
	insert_list(&mut ctx, "link_flags", &component.link_flags);
	insert_str(&mut ctx, "platform_os", &component.platform_os);
	insert_str(&mut ctx, "platform_arch", &component.platform_arch);
	insert_str(&mut ctx, "platform_abi", &component.platform_abi);

	let mut env = Map::new();
	for (k, v) in &component.env {
		env.insert(k.as_str().into(), Dynamic::from(v.clone()));
	}
	ctx.insert("env".into(), Dynamic::from(env));
	ctx.insert("profile".into(), Dynamic::from(profile_to_map(&component.profile)));
	ctx
}

fn profile_to_map(profile: &ProfileView) -> Map {
	let mut m = Map::new();
	insert_str(&mut m, "name", &profile.name);
	m.insert("opt_level".into(), Dynamic::from(profile.opt_level));
	m.insert("is_debug".into(), Dynamic::from(profile.debug));
	m.insert("lto".into(), Dynamic::from(profile.lto));
	m.insert("strip".into(), Dynamic::from(profile.strip));
	m.insert("coverage".into(), Dynamic::from(profile.coverage));
	insert_list(&mut m, "defines", &profile.defines);
	insert_list(&mut m, "sanitizers", &profile.sanitizers);
	m
}

fn insert_str(map: &mut Map, key: &str, value: &str) {
	map.insert(key.into(), Dynamic::from(value.to_string()));
}

fn insert_list(map: &mut Map, key: &str, values: &[String]) {
	let list: rhai::Array = values.iter().map(|v| Dynamic::from(v.clone())).collect();
	map.insert(key.into(), Dynamic::from(list));
}

fn parse_action(spec: Map) -> Result<ActionDecl, Box<EvalAltResult>> {
	let string_field = |key: &str| -> Result<String, Box<EvalAltResult>> {
		match spec.get(key) {
			Some(v) if v.is_string() => Ok(v.clone().into_string().expect("checked string")),
			_ => Err(format!("action requires string `{key}`").into()),
		}
	};
	let list_field = |key: &str| -> Result<Vec<String>, Box<EvalAltResult>> {
		match spec.get(key) {
			Some(v) if v.is_array() => v
				.clone()
				.try_cast::<rhai::Array>()
				.expect("checked array")
				.into_iter()
				.map(|item| item.into_string().map_err(|_| format!("`{key}` expects strings").into()))
				.collect(),
			None => Ok(Vec::new()),
			_ => Err(format!("action field `{key}` expects an array").into()),
		}
	};

	let outputs = list_field("outputs")?
		.into_iter()
		.map(|entry| {
			let is_dir = entry.ends_with('/');
			(entry.trim_end_matches('/').to_string(), is_dir)
		})
		.collect();

	let mut env = BTreeMap::new();
	if let Some(v) = spec.get("env") {
		let map = v.clone().try_cast::<Map>().ok_or("`env` expects a map")?;
		for (k, value) in map {
			env.insert(
				k.to_string(),
				value.into_string().map_err(|_| "`env` values must be strings")?,
			);
		}
	}

	let toolchain_id = match spec.get("toolchain_id") {
		Some(v) if v.is_string() => Some(v.clone().into_string().expect("checked string")),
		_ => None,
	};

	Ok(ActionDecl {
		name: string_field("name")?,
		command: string_field("command")?,
		args: list_field("args")?,
		inputs: list_field("inputs")?,
		outputs,
		env,
		toolchain_id,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hooks() -> CellHooks {
		CellHooks {
			obj_path: Box::new(|src| format!("forge-out/obj/{src}.o")),
			prior_depfile_headers: Box::new(|_| vec![]),
			lib_path: Box::new(|name| format!("forge-out/lib/lib{name}.a")),
			bin: Box::new(|name| format!("/tools/bin/{name}")),
			tool_id: Box::new(|| "gcc@abc123".into()),
			read_file: Box::new(|_| Err("not implemented in tests".into())),
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
}
