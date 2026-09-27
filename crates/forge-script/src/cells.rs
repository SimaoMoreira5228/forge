use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use forge_diagnostics::{ForgeDiagnostic, codes};
use rhai::{Dynamic, EvalAltResult, Map};

#[derive(Debug, Clone)]
pub struct ActionDecl {
	pub name: String,
	pub configuration: forge_core::ConfigTransition,
	pub command: String,
	pub args: Vec<String>,
	pub inputs: Vec<PathBufArg>,
	pub execution_deps: Vec<PathBufArg>,
	pub outputs: Vec<(PathBufArg, bool)>,
	pub artifact: Option<String>,
	pub workdir: Option<String>,
	pub stdout: Option<String>,
	pub is_test: bool,
	pub compile_command: Option<String>,
	pub environment_files: Vec<(String, String, Option<String>, Vec<String>)>,
	pub argument_files: Vec<(String, String, String, Option<String>)>,
	pub env: BTreeMap<String, String>,
	pub toolchain_id: Option<String>,
}

type PathBufArg = String;

#[derive(Debug, Clone, Default)]
pub struct ComponentView {
	pub session: CellSession,
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
	pub linker: String,
	pub link_flags: Vec<String>,
	pub metadata: toml::Table,
}

#[derive(Debug, Clone, Default)]
pub struct CellSession {
	pub fetched_sources: Vec<FetchedSource>,
	pub workspace: String,
	pub platform_os: String,
	pub platform_arch: String,
	pub platform_abi: String,
	pub profile: ProfileView,
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
type FileReader = Box<dyn Fn(&str) -> Result<String, String>>;
type Globber = Box<dyn Fn(&str) -> Result<Vec<String>, String>>;

const CLAIMED_SECTION: &str = "forge-claimed";

pub struct WorkspaceHooks {
	pub read_file: FileReader,
	pub glob: Globber,
}

pub struct CellHooks {
	pub workspace: WorkspaceHooks,
	pub artifact_path: ArtifactPath,
	pub depfile_inputs: Globber,
	pub lib_path: Box<dyn Fn(&str) -> String>,
	pub bin: Box<dyn Fn(&str) -> String>,
	pub tool_id: Box<dyn Fn() -> String>,
}

#[derive(Clone, Default)]
pub struct CellPlan {
	value: Rc<RefCell<Dynamic>>,
}

impl CellPlan {
	pub fn new(value: Dynamic) -> Self {
		Self {
			value: Rc::new(RefCell::new(value)),
		}
	}

	pub fn section(&self, name: &str) -> Dynamic {
		let value = self.value.borrow();
		match value.as_map_ref() {
			Ok(plan) => entry(&plan, name),
			Err(_) => Dynamic::UNIT,
		}
	}

	pub fn entry(&self, section: &str, key: &str) -> Dynamic {
		let value = self.value.borrow();
		let Ok(plan) = value.as_map_ref() else {
			return Dynamic::UNIT;
		};
		let Some(section) = plan.get(section) else {
			return Dynamic::UNIT;
		};
		let Ok(entries) = section.as_map_ref() else {
			return Dynamic::UNIT;
		};
		entries.get(key).cloned().unwrap_or(Dynamic::UNIT)
	}

	pub fn claim(&self, responsibility: &str) -> bool {
		let mut value = self.value.borrow_mut();
		let claimed = value.as_map_mut().map_err(|_| "cell plan is not a map").map(|mut plan| {
			let mut claimed = match plan.get(CLAIMED_SECTION) {
				Some(list) => list.clone().try_cast::<rhai::Array>().unwrap_or_default(),
				None => rhai::Array::new(),
			};
			let already = claimed
				.iter()
				.any(|name| name.is_string() && name.clone_cast::<String>() == responsibility);
			if !already {
				claimed.push(Dynamic::from(responsibility.to_string()));
				plan.insert(CLAIMED_SECTION.into(), Dynamic::from(claimed));
			}
			!already
		});
		claimed.unwrap_or(false)
	}
}

fn entry(map: &Map, key: &str) -> Dynamic {
	map.get(key).cloned().unwrap_or(Dynamic::UNIT)
}

pub fn plan(script: &str, session: &CellSession, hooks: WorkspaceHooks) -> Result<CellPlan, ForgeDiagnostic> {
	let mut engine = new_engine();
	register_workspace(&mut engine, hooks);
	register_memo(&mut engine);
	register_graph_ops(&mut engine);
	let ast = cell_script(&mut engine, script)?;
	let mut scope = rhai::Scope::new();
	let value = engine
		.call_fn::<Dynamic>(&mut scope, &ast, "plan", (context_map(session, None),))
		.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("cell plan failed: {e}")))?;
	Ok(CellPlan::new(value))
}

pub fn lower(
	script: &str,
	plan: &CellPlan,
	component: &ComponentView,
	hooks: CellHooks,
) -> Result<Vec<ActionDecl>, ForgeDiagnostic> {
	let actions: std::rc::Rc<std::cell::RefCell<Vec<ActionDecl>>> = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
	let mut engine = new_engine();
	register_emitters(&mut engine, &actions);
	register_plan_ops(&mut engine);
	register_component(&mut engine, hooks);
	register_memo(&mut engine);
	register_graph_ops(&mut engine);

	let ast = cell_script(&mut engine, script)?;
	let mut scope = rhai::Scope::new();
	engine
		.call_fn::<()>(
			&mut scope,
			&ast,
			"build",
			(context_map(&component.session, Some(component)), Dynamic::from(plan.clone())),
		)
		.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("cell failed: {e}")))?;

	Ok(std::mem::take(&mut *actions.borrow_mut()))
}

fn new_engine() -> rhai::Engine {
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	engine.set_max_call_levels(128);
	engine
}

fn cell_script(engine: &mut rhai::Engine, script: &str) -> Result<rhai::AST, ForgeDiagnostic> {
	engine
		.compile(script)
		.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("cell failed to parse: {e}")))
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

fn register_workspace(engine: &mut rhai::Engine, hooks: WorkspaceHooks) {
	let WorkspaceHooks { read_file, glob } = hooks;
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
	engine.register_fn(
		"glob",
		move |_ctx: &mut Map, pattern: &str| -> Result<rhai::Array, Box<EvalAltResult>> {
			let files = glob(pattern).map_err(|e| -> Box<EvalAltResult> { e.into() })?;
			Ok(files.into_iter().map(Dynamic::from).collect())
		},
	);
}

fn register_plan_ops(engine: &mut rhai::Engine) {
	engine.register_type_with_name::<CellPlan>("CellPlan");
	engine.register_fn("get", |plan: &mut CellPlan, section: &str| plan.section(section));
	engine.register_fn("get", |plan: &mut CellPlan, section: &str, key: &str| {
		plan.entry(section, key)
	});
	engine.register_fn("once", |plan: &mut CellPlan, responsibility: &str| plan.claim(responsibility));
}

fn register_memo(engine: &mut rhai::Engine) {
	let memo: Rc<RefCell<BTreeMap<String, Dynamic>>> = Rc::new(RefCell::new(BTreeMap::new()));
	let reads = Rc::clone(&memo);
	engine.register_fn("memo_get", move |key: &str| -> Dynamic {
		match reads.borrow().get(key) {
			Some(value) => value.clone(),
			None => Dynamic::UNIT,
		}
	});
	let writes = Rc::clone(&memo);
	engine.register_fn("memo_put", move |key: &str, value: Dynamic| {
		writes.borrow_mut().insert(key.to_string(), value);
	});
}

fn register_component(engine: &mut rhai::Engine, hooks: CellHooks) {
	let CellHooks {
		workspace,
		artifact_path,
		depfile_inputs,
		lib_path,
		bin,
		tool_id,
	} = hooks;
	register_workspace(engine, workspace);

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
}

fn register_graph_ops(engine: &mut rhai::Engine) {
	use crate::dep_graph;

	engine.register_fn("graph_roots", |adjacency: &mut Map| -> rhai::Array {
		dep_graph::keys_to_rhai(dep_graph::roots(&dep_graph::adjacency_from_rhai(adjacency)))
	});
	engine.register_fn("graph_reachable", |adjacency: &mut Map, roots: rhai::Array| -> rhai::Array {
		let roots: Vec<String> = roots.into_iter().filter_map(|root| root.into_string().ok()).collect();
		dep_graph::keys_to_rhai(dep_graph::reachable(&dep_graph::adjacency_from_rhai(adjacency), &roots))
	});
	engine.register_fn("graph_transitive", |adjacency: &mut Map, key: &str| -> rhai::Array {
		dep_graph::keys_to_rhai(dep_graph::transitive(&dep_graph::adjacency_from_rhai(adjacency), key))
	});
	engine.register_fn("graph_reverse", |adjacency: &mut Map| -> Map {
		let mut out = Map::new();
		for (key, list) in dep_graph::reverse(&dep_graph::adjacency_from_rhai(adjacency)) {
			out.insert(key.into(), Dynamic::from(dep_graph::keys_to_rhai(list)));
		}
		out
	});
	engine.register_fn("graph_topo", |adjacency: &mut Map| -> rhai::Array {
		dep_graph::keys_to_rhai(dep_graph::toposort(&dep_graph::adjacency_from_rhai(adjacency)))
	});
}

fn context_map(session: &CellSession, component: Option<&ComponentView>) -> Map {
	let mut ctx = Map::new();
	insert_str(&mut ctx, "workspace", &session.workspace);
	insert_str(&mut ctx, "platform_os", &session.platform_os);
	insert_str(&mut ctx, "platform_arch", &session.platform_arch);
	insert_str(&mut ctx, "platform_abi", &session.platform_abi);
	ctx.insert("profile".into(), Dynamic::from(profile_to_map(&session.profile)));
	ctx.insert(
		"cell_config".into(),
		crate::rhai_rt::toml_value_to_dynamic(toml::Value::Table(session.cell_config.clone())).expect("valid TOML value"),
	);
	let targets: rhai::Array = session
		.targets
		.iter()
		.map(|table| crate::rhai_rt::toml_value_to_dynamic(toml::Value::Table(table.clone())).expect("valid TOML value"))
		.collect();
	ctx.insert("targets".into(), Dynamic::from(targets));

	let fetched_sources: rhai::Array = session
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

	let Some(component) = component else {
		return ctx;
	};
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
	insert_str(&mut ctx, "linker", &component.linker);
	insert_list(&mut ctx, "link_flags", &component.link_flags);
	let artifact_list: rhai::Array = component
		.dep_artifacts
		.iter()
		.map(|(name, path)| Dynamic::from(vec![Dynamic::from(name.clone()), Dynamic::from(path.clone())]))
		.collect();
	ctx.insert("dep_artifacts".into(), Dynamic::from(artifact_list));
	let mut env = Map::new();
	for (k, v) in &component.env {
		env.insert(k.as_str().into(), Dynamic::from(v.clone()));
	}
	ctx.insert("env".into(), Dynamic::from(env));
	ctx.insert(
		"metadata".into(),
		crate::rhai_rt::toml_value_to_dynamic(toml::Value::Table(component.metadata.clone())).expect("valid TOML value"),
	);
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
	let configuration = match spec.get("configuration") {
		None => forge_core::ConfigTransition::Target,
		Some(v) if v.is_string() => {
			let name = v.clone().into_string().expect("checked string");
			forge_core::ConfigTransition::parse(&name)
				.ok_or_else(|| Box::<EvalAltResult>::from(format!("unknown configuration `{name}`")))?
		}
		Some(_) => return Err("action `configuration` expects a string".into()),
	};
	let workdir = match spec.get("workdir") {
		Some(v) if v.is_string() => Some(v.clone().into_string().expect("checked string")),
		_ => None,
	};
	let stdout = spec.get("stdout").and_then(|v| v.clone().into_string().ok());
	let is_test = spec
		.get("is_test")
		.is_some_and(|v| v.clone().try_cast::<bool>().unwrap_or(false));
	let compile_command = spec.get("compile_command").and_then(|v| v.clone().into_string().ok());
	let artifact = match spec.get("artifact") {
		Some(v) if v.is_string() => Some(v.clone().into_string().expect("checked string")),
		Some(_) => return Err("action `artifact` expects a string".into()),
		None => None,
	};
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
		configuration,
		command: string_field("command")?,
		args: list_field("args")?,
		inputs: list_field("inputs")?,
		execution_deps,
		outputs,
		artifact,
		workdir,
		stdout,
		is_test,
		compile_command,
		environment_files,
		argument_files,
		env,
		toolchain_id,
	})
}

#[cfg(test)]
mod rust_prelude;
#[cfg(test)]
mod tests;
