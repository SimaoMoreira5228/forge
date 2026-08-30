use std::collections::BTreeMap;

use forge_diagnostics::{ForgeDiagnostic, codes};
use rhai::{Dynamic, EvalAltResult, Map};

#[derive(Debug, Clone)]
pub struct ActionDecl {
	pub name: String,
	pub command: String,
	pub args: Vec<String>,
	pub inputs: Vec<PathBufArg>,
	pub execution_deps: Vec<PathBufArg>,
	pub outputs: Vec<(PathBufArg, bool)>,
	pub workdir: Option<String>,
	pub stdout: Option<String>,
	pub environment_files: Vec<(String, String, Option<String>, Vec<String>)>,
	pub argument_files: Vec<(String, String, String, Option<String>)>,
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
	pub fetched_sources: Vec<FetchedSource>,
	pub fetch_owner: bool,
	pub workspace: String,
	pub linker: String,
	pub link_flags: Vec<String>,
	pub platform_os: String,
	pub platform_arch: String,
	pub platform_abi: String,
	pub profile: ProfileView,
	pub metadata: toml::Table,
	pub cell_config: toml::Table,
	pub targets: Vec<toml::Table>,
}

#[derive(Debug, Clone, Default)]
pub struct FetchedSource {
	pub name: String,
	pub version: String,
	pub root: String,
	pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileView {
	pub name: String,
	pub opt_level: String,
	pub debug: String,
	pub lto: String,
	pub strip: String,
	pub coverage: bool,
	pub defines: Vec<String>,
	pub sanitizers: Vec<String>,
	pub options: toml::Table,
	pub build: Option<Box<ProfileView>>,
}

impl From<&forge_core::Profile> for ProfileView {
	fn from(profile: &forge_core::Profile) -> Self {
		Self {
			name: profile.name.clone(),
			opt_level: profile.opt_level.as_str().into(),
			debug: profile.debug.as_str().into(),
			lto: profile.lto.as_str().into(),
			strip: profile.strip.as_str().into(),
			coverage: profile.coverage,
			defines: profile.defines.clone(),
			sanitizers: profile.sanitizers.clone(),
			options: profile.options.clone(),
			build: profile.build.as_deref().map(|build| Box::new(ProfileView::from(build))),
		}
	}
}

type ArtifactPath = Box<dyn Fn(&str, &str) -> Result<String, String>>;

pub struct CellHooks {
	pub artifact_path: ArtifactPath,
	pub depfile_inputs: Globber,
	pub lib_path: Box<dyn Fn(&str) -> String>,
	pub bin: Box<dyn Fn(&str) -> String>,
	pub tool_id: Box<dyn Fn() -> String>,
	pub read_file: FileReader,
	pub glob: Globber,
}
type FileReader = Box<dyn Fn(&str) -> Result<String, String>>;
type Globber = Box<dyn Fn(&str) -> Result<Vec<String>, String>>;

pub fn lower(script: &str, component: &ComponentView, hooks: CellHooks) -> Result<Vec<ActionDecl>, ForgeDiagnostic> {
	let actions: std::rc::Rc<std::cell::RefCell<Vec<ActionDecl>>> = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	engine.set_max_call_levels(128);

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
		artifact_path,
		depfile_inputs,
		lib_path,
		bin,
		tool_id,
		read_file,
		glob,
	} = hooks;

	engine.register_fn("bin", move |_ctx: &mut Map, name: &str| -> String { bin(name) });
	engine.register_fn("tool_id", move |_ctx: &mut Map| -> String { tool_id() });
	engine.register_fn(
		"lib_path",
		move |_ctx: &mut Map, filename: &str| -> Result<String, Box<EvalAltResult>> {
			if filename.is_empty() || matches!(filename, "." | "..") || filename.contains(['/', '\\', ':', '\0']) {
				return Err("lib_path requires a filename without path separators".into());
			}
			Ok(lib_path(filename))
		},
	);
	engine.register_fn(
		"artifact_path",
		move |_ctx: &mut Map, src: &str, category: &str| -> Result<String, Box<EvalAltResult>> {
			artifact_path(src, category).map_err(Into::into)
		},
	);
	engine.register_fn(
		"depfile_inputs",
		move |_ctx: &mut Map, path: &str| -> Result<rhai::Array, Box<EvalAltResult>> {
			let inputs = depfile_inputs(path).map_err(Box::<EvalAltResult>::from)?;
			Ok(inputs.into_iter().map(Dynamic::from).collect())
		},
	);
	engine.register_fn(
		"read_file",
		move |_ctx: &mut Map, path: &str| -> Result<String, Box<EvalAltResult>> { read_file(path).map_err(|e| e.into()) },
	);
	engine.register_fn("json_decode", |text: &str| -> Result<Dynamic, Box<EvalAltResult>> {
		crate::rhai_rt::json_decode(text).map_err(Into::into)
	});
	engine.register_fn("toml_decode", |text: &str| -> Result<Map, Box<EvalAltResult>> {
		crate::rhai_rt::toml_decode(text).map_err(Into::into)
	});
	register_graph_ops(engine);
	engine.register_fn(
		"glob",
		move |_ctx: &mut Map, pattern: &str| -> Result<rhai::Array, Box<EvalAltResult>> {
			let files = glob(pattern).map_err(|e| -> Box<EvalAltResult> { e.into() })?;
			Ok(files.into_iter().map(Dynamic::from).collect())
		},
	);
	let _ = component.profile;
}

fn register_graph_ops(engine: &mut rhai::Engine) {
	use crate::dep_graph;

	engine.register_fn("graph_roots", |adjacency: Map| -> rhai::Array {
		dep_graph::keys_to_rhai(dep_graph::roots(&dep_graph::adjacency_from_rhai(&adjacency)))
	});
	engine.register_fn("graph_reachable", |adjacency: Map, roots: rhai::Array| -> rhai::Array {
		let roots: Vec<String> = roots.into_iter().filter_map(|root| root.into_string().ok()).collect();
		dep_graph::keys_to_rhai(dep_graph::reachable(&dep_graph::adjacency_from_rhai(&adjacency), &roots))
	});
	engine.register_fn("graph_transitive", |adjacency: Map, key: &str| -> rhai::Array {
		dep_graph::keys_to_rhai(dep_graph::transitive(&dep_graph::adjacency_from_rhai(&adjacency), key))
	});
	engine.register_fn("graph_reverse", |adjacency: Map| -> Map {
		let mut out = Map::new();
		for (key, list) in dep_graph::reverse(&dep_graph::adjacency_from_rhai(&adjacency)) {
			out.insert(key.into(), Dynamic::from(dep_graph::keys_to_rhai(list)));
		}
		out
	});
	engine.register_fn("graph_topo", |adjacency: Map| -> rhai::Array {
		dep_graph::keys_to_rhai(dep_graph::toposort(&dep_graph::adjacency_from_rhai(&adjacency)))
	});
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
	let fetched_sources: rhai::Array = component
		.fetched_sources
		.iter()
		.map(|package| {
			let mut value = Map::new();
			insert_str(&mut value, "name", &package.name);
			insert_str(&mut value, "version", &package.version);
			insert_str(&mut value, "root", &package.root);
			let dependencies: rhai::Array = package.dependencies.iter().map(|name| Dynamic::from(name.clone())).collect();
			value.insert("dependencies".into(), Dynamic::from(dependencies));
			Dynamic::from(value)
		})
		.collect();
	ctx.insert("fetched_sources".into(), Dynamic::from(fetched_sources));
	ctx.insert("fetch_owner".into(), Dynamic::from(component.fetch_owner));
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
	for (name, table) in [("metadata", &component.metadata), ("cell_config", &component.cell_config)] {
		ctx.insert(
			name.into(),
			crate::rhai_rt::toml_value_to_dynamic(toml::Value::Table(table.clone())).expect("valid TOML value"),
		);
	}
	let targets: rhai::Array = component
		.targets
		.iter()
		.map(|table| crate::rhai_rt::toml_value_to_dynamic(toml::Value::Table(table.clone())).expect("valid TOML value"))
		.collect();
	ctx.insert("targets".into(), Dynamic::from(targets));
	ctx
}

fn profile_to_map(profile: &ProfileView) -> Map {
	let mut m = Map::new();
	insert_str(&mut m, "name", &profile.name);
	insert_str(&mut m, "opt_level", &profile.opt_level);
	insert_str(&mut m, "debug", &profile.debug);
	m.insert("is_debug".into(), Dynamic::from(profile.debug != "none"));
	insert_str(&mut m, "lto", &profile.lto);
	insert_str(&mut m, "strip", &profile.strip);
	m.insert("coverage".into(), Dynamic::from(profile.coverage));
	insert_list(&mut m, "defines", &profile.defines);
	insert_list(&mut m, "sanitizers", &profile.sanitizers);
	m.insert(
		"options".into(),
		crate::rhai_rt::toml_value_to_dynamic(toml::Value::Table(profile.options.clone())).expect("valid TOML value"),
	);
	if let Some(build) = &profile.build {
		m.insert("build".into(), Dynamic::from(profile_to_map(build)));
	}
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
	let execution_deps = list_field("execution_deps")?;

	let mut env = BTreeMap::new();
	if let Some(v) = spec.get("env") {
		let map = v.clone().try_cast::<Map>().ok_or("`env` expects a map")?;
		for (k, value) in map {
			env.insert(
				k.to_string(),
				value
					.into_string()
					.map_err(|_| format!("`env` value `{k}` must be a string"))?,
			);
		}
	}

	let toolchain_id = match spec.get("toolchain_id") {
		Some(v) if v.is_string() => Some(v.clone().into_string().expect("checked string")),
		_ => None,
	};
	let workdir = match spec.get("workdir") {
		Some(v) if v.is_string() => Some(v.clone().into_string().expect("checked string")),
		_ => None,
	};
	let stdout = spec.get("stdout").and_then(|v| v.clone().into_string().ok());
	let mut environment_files = Vec::new();
	if let Some(value) = spec.get("environment_files") {
		let map = value.clone().try_cast::<Map>().ok_or("`environment_files` expects a map")?;
		for (key, value) in map {
			let value = if value.is_string() {
				(
					value.into_string().map_err(|_| "environment file paths must be strings")?,
					None,
					Vec::new(),
				)
			} else {
				let map = value
					.try_cast::<Map>()
					.ok_or("environment file declarations must be strings or maps")?;
				let path = map
					.get("path")
					.ok_or("environment file declaration needs `path`")?
					.clone()
					.into_string()
					.map_err(|_| "environment file paths must be strings")?;
				let line_prefix = map.get("line_prefix").and_then(|v| v.clone().into_string().ok());
				let ignored_keys = map
					.get("ignored_keys")
					.map(|v| {
						v.clone()
							.try_cast::<rhai::Array>()
							.ok_or("environment file `ignored_keys` must be a list")
					})
					.transpose()?
					.unwrap_or_default()
					.into_iter()
					.map(|v| v.into_string().map_err(|_| "environment file ignored keys must be strings"))
					.collect::<Result<Vec<_>, _>>()?;
				(path, line_prefix, ignored_keys)
			};
			environment_files.push((key.to_string(), value.0, value.1, value.2));
		}
	}

	let mut argument_files = Vec::new();
	if let Some(value) = spec.get("argument_files") {
		let entries = value
			.clone()
			.try_cast::<rhai::Array>()
			.ok_or("`argument_files` expects an array")?;
		for entry in entries {
			let map = entry.try_cast::<Map>().ok_or("argument file entries must be maps")?;
			let string = |key: &str| -> Result<String, Box<EvalAltResult>> {
				let value = map
					.get(key)
					.ok_or_else(|| Box::<EvalAltResult>::from(format!("argument file needs `{key}`")))?;
				value
					.clone()
					.into_string()
					.map_err(|_| Box::<EvalAltResult>::from(format!("argument file `{key}` must be a string")))
			};
			argument_files.push((
				string("path")?,
				string("prefix")?,
				string("flag")?,
				map.get("root_marker").and_then(|value| value.clone().into_string().ok()),
			));
		}
	}

	Ok(ActionDecl {
		name: string_field("name")?,
		command: string_field("command")?,
		args: list_field("args")?,
		inputs: list_field("inputs")?,
		execution_deps,
		outputs,
		workdir,
		stdout,
		environment_files,
		argument_files,
		env,
		toolchain_id,
	})
}

#[cfg(test)]
mod tests;
