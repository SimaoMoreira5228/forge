use crate::graph::{BuildGraph, Component, ComponentRef, DependencyEdge, DotOptions, Target};
use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct GraphApi {
	graph: Arc<Mutex<BuildGraph>>,
	project_root: PathBuf,
	current_package: Arc<Mutex<String>>,
}

impl GraphApi {
	pub fn new(project_root: PathBuf, graph: Arc<Mutex<BuildGraph>>) -> Self {
		Self {
			graph,
			project_root,
			current_package: Arc::new(Mutex::new(String::from("."))),
		}
	}

	pub fn get_graph(&self) -> Arc<Mutex<BuildGraph>> {
		self.graph.clone()
	}
}

impl UserData for GraphApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "graph")]
impl GraphApi {
	pub fn create(project_root: PathBuf) -> Self {
		Self::new(project_root, Arc::new(Mutex::new(BuildGraph::new())))
	}

	/// Set the current package being evaluated (called by Project loader)
	pub fn set_package(&self, path: String) {
		log::debug!("Setting current package to: '{}'", path);
		let mut pkg = self.current_package.lock().unwrap();
		*pkg = path;
	}

	/// Register a target platform for builds
	pub fn target(&self, tbl: Table) -> Result<Value> {
		let name: String = tbl.get("name")?;
		let triple: String = tbl.get("triple").unwrap_or_else(|_| infer_triple_from_name(&name));

		let target = Target::new(&name, &triple);

		let mut g = self.graph.lock().unwrap();
		g.add_target(target).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		Ok(Value::Nil)
	}

	/// Define a library component
	pub fn library(&self, tbl: Table) -> Result<Value> {
		let name: String = tbl.get("name")?;
		let target: String = tbl.get("target").or_else(|_| {
			let g = self.graph.lock().unwrap();
			g.targets()
				.next()
				.map(|t| t.name.clone())
				.ok_or_else(|| mlua::Error::RuntimeError("No targets defined. Call forge.graph.target() first.".into()))
		})?;

		let sources: Vec<String> = tbl.get("sources").or_else(|_| tbl.get("srcs")).unwrap_or_default();
		let include_dirs: Vec<String> = tbl.get("include_dirs").or_else(|_| tbl.get("includes")).unwrap_or_default();

		let mut component = Component::library(&name, &target)
			.with_sources(sources.iter().map(PathBuf::from).collect())
			.with_include_dirs(include_dirs.iter().map(PathBuf::from).collect());

		// Set package ID from current context
		{
			let pkg = self.current_package.lock().unwrap();
			component = component.with_package(crate::graph::PackageId::new(pkg.as_str()));
		}

		if let Ok(vis_val) = tbl.get::<Value>("visibility") {
			component = component.with_visibility(parse_visibility(vis_val)?);
		}

		if let Ok(comp_val) = tbl.get::<Vec<String>>("compatible_with") {
			component = component.with_compatible_with(comp_val.into_iter().map(crate::graph::ConstraintRef::new).collect());
		}

		if let Ok(defines) = tbl.get::<Vec<String>>("defines") {
			component = component.with_defines(defines.into_iter().map(|s| (s, None)).collect());
		}
		if let Ok(cflags) = tbl.get::<Vec<String>>("cflags") {
			component = component.with_compiler_flags(cflags);
		}

		if let Ok(exec_cfg_str) = tbl.get::<String>("exec_cfg") {
			let exec_cfg = match exec_cfg_str.as_str() {
				"host" => crate::graph::ConfigTransition::Host,
				"exec" => crate::graph::ConfigTransition::Exec,
				"target" => crate::graph::ConfigTransition::Target,
				_ => return Err(mlua::Error::RuntimeError(format!("Invalid exec_cfg: {}", exec_cfg_str))),
			};
			component = component.with_exec_cfg(Some(exec_cfg));
		}

		if let Ok(outputs) = tbl.get::<Vec<String>>("outputs") {
			component = component.with_outputs(outputs.into_iter().map(PathBuf::from).collect());
		}

		if let Ok(env_table) = tbl.get::<Table>("env") {
			let mut env_map = std::collections::HashMap::new();
			for pair in env_table.pairs::<String, String>() {
				let (k, v) = pair?;
				env_map.insert(k, v);
			}
			component = component.with_env(env_map);
		}

		let mut g = self.graph.lock().unwrap();
		let comp_id = g
			.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		if let Ok(deps) = tbl.get::<Value>("deps") {
			match deps {
				Value::Table(deps_tbl) => {
					for pair in deps_tbl.pairs::<Value, Value>() {
						let (_key, val) = pair?;
						let (dep_ref, edge) = parse_dependency(val)?;
						g.add_dependency(comp_id, dep_ref, edge)
							.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
					}
				}
				_ => return Err(mlua::Error::RuntimeError("deps must be a list of strings or tables".into())),
			}
		}

		Ok(Value::Nil)
	}

	/// Define a binary component (executable)
	pub fn binary(&self, tbl: Table) -> Result<Value> {
		let name: String = tbl.get("name")?;
		let target: String = tbl.get("target").or_else(|_| {
			let g = self.graph.lock().unwrap();
			g.targets()
				.next()
				.map(|t| t.name.clone())
				.ok_or_else(|| mlua::Error::RuntimeError("No targets defined. Call forge.graph.target() first.".into()))
		})?;

		let sources: Vec<String> = tbl.get("sources").or_else(|_| tbl.get("srcs")).unwrap_or_default();
		let include_dirs: Vec<String> = tbl.get("include_dirs").or_else(|_| tbl.get("includes")).unwrap_or_default();

		let mut component = Component::binary(&name, &target)
			.with_sources(sources.iter().map(PathBuf::from).collect())
			.with_include_dirs(include_dirs.iter().map(PathBuf::from).collect());

		// Set package ID from current context
		{
			let pkg = self.current_package.lock().unwrap();
			component = component.with_package(crate::graph::PackageId::new(pkg.as_str()));
		}

		if let Ok(vis_val) = tbl.get::<Value>("visibility") {
			component = component.with_visibility(parse_visibility(vis_val)?);
		}

		if let Ok(comp_val) = tbl.get::<Vec<String>>("compatible_with") {
			component = component.with_compatible_with(comp_val.into_iter().map(crate::graph::ConstraintRef::new).collect());
		}

		if let Ok(defines) = tbl.get::<Vec<String>>("defines") {
			component = component.with_defines(defines.into_iter().map(|s| (s, None)).collect());
		}
		if let Ok(cflags) = tbl.get::<Vec<String>>("cflags") {
			component = component.with_compiler_flags(cflags);
		}
		if let Ok(ldflags) = tbl.get::<Vec<String>>("ldflags") {
			component = component.with_linker_flags(ldflags);
		}
		if let Ok(system_libs) = tbl.get::<Vec<String>>("system_libs") {
			component = component.with_system_libs(system_libs);
		}

		if let Ok(exec_cfg_str) = tbl.get::<String>("exec_cfg") {
			let exec_cfg = match exec_cfg_str.as_str() {
				"host" => crate::graph::ConfigTransition::Host,
				"exec" => crate::graph::ConfigTransition::Exec,
				"target" => crate::graph::ConfigTransition::Target,
				_ => {
					return Err(mlua::Error::RuntimeError(format!(
						"Invalid exec_cfg: {}",
						exec_cfg_str
					)))
				}
			};
			component = component.with_exec_cfg(Some(exec_cfg));
		}

		if let Ok(outputs) = tbl.get::<Vec<String>>("outputs") {
			component = component.with_outputs(outputs.into_iter().map(PathBuf::from).collect());
		}

		if let Ok(env_table) = tbl.get::<Table>("env") {
			let mut env_map = std::collections::HashMap::new();
			for pair in env_table.pairs::<String, String>() {
				let (k, v) = pair?;
				env_map.insert(k, v);
			}
			component = component.with_env(env_map);
		}

		let mut g = self.graph.lock().unwrap();
		let comp_id = g
			.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		if let Ok(deps) = tbl.get::<Value>("deps") {
			match deps {
				Value::Table(deps_tbl) => {
					for pair in deps_tbl.pairs::<Value, Value>() {
						let (_key, val) = pair?;
						let (dep_ref, edge) = parse_dependency(val)?;
						g.add_dependency(comp_id, dep_ref, edge)
							.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
					}
				}
				_ => return Err(mlua::Error::RuntimeError("deps must be a list of strings or tables".into())),
			}
		}

		Ok(Value::Nil)
	}

	/// Define a custom component with arbitrary command
	pub fn custom(&self, tbl: Table) -> Result<Value> {
		let name: String = tbl.get("name")?;
		let target: String = tbl.get("target").or_else(|_| {
			let g = self.graph.lock().unwrap();
			g.targets()
				.next()
				.map(|t| t.name.clone())
				.ok_or_else(|| mlua::Error::RuntimeError("No targets defined. Call forge.graph.target() first.".into()))
		})?;
		let command: String = tbl.get("command")?;
		let args: Vec<String> = tbl.get("args").unwrap_or_default();
		let sources: Vec<String> = tbl.get("sources").or_else(|_| tbl.get("srcs")).unwrap_or_default();
		let outputs: Vec<String> = tbl.get("outputs").unwrap_or_default();

		let mut component = Component::custom(&name, &target, command, args)
			.with_sources(sources.iter().map(PathBuf::from).collect())
			.with_outputs(outputs.iter().map(PathBuf::from).collect());

		// Set package ID from current context
		{
			let pkg = self.current_package.lock().unwrap();
			component = component.with_package(crate::graph::PackageId::new(pkg.as_str()));
		}

		if let Ok(vis_val) = tbl.get::<Value>("visibility") {
			component = component.with_visibility(parse_visibility(vis_val)?);
		}

		if let Ok(env_table) = tbl.get::<Table>("env") {
			let mut env_map = std::collections::HashMap::new();
			for pair in env_table.pairs::<String, String>() {
				let (k, v) = pair?;
				env_map.insert(k, v);
			}
			component = component.with_env(env_map);
		}

		let mut g = self.graph.lock().unwrap();
		let comp_id = g
			.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		if let Ok(deps) = tbl.get::<mlua::Value>("deps") {
			match deps {
				mlua::Value::Table(deps_tbl) => {
					for pair in deps_tbl.pairs::<mlua::Value, mlua::Value>() {
						let (_key, val) = pair?;
						let (dep_ref, edge) = parse_dependency(val)?;
						g.add_dependency(comp_id, dep_ref, edge)
							.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
					}
				}
				_ => return Err(mlua::Error::RuntimeError("deps must be a list of strings or tables".into())),
			}
		}

		Ok(mlua::Value::Nil)
	}

	/// Define a test component
	pub fn test(&self, tbl: Table) -> Result<Value> {
		let name: String = tbl.get("name")?;
		let target: String = tbl.get("target").unwrap_or_else(|_| {
			let first_target = {
				let g = self.graph.lock().unwrap();
				g.targets().next().map(|t| t.name.clone())
			};
			match first_target {
				Some(t) => t,
				None => {
					// Auto-register a host target so bash/script tests work without preamble
					let host_triple = if cfg!(target_os = "windows") {
						"x86_64-pc-windows-msvc"
					} else if cfg!(target_os = "macos") {
						"aarch64-apple-darwin"
					} else {
						"x86_64-unknown-linux-gnu"
					};
					let host_target = crate::graph::Target::new("host", host_triple);
					let mut g = self.graph.lock().unwrap();
					let _ = g.add_target(host_target);
					"host".to_string()
				}
			}
		});

		// Binary path (e.g. "forge-out/linux_x64/debug/test/my_test")
		let executable: Option<PathBuf> = tbl.get::<String>("binary").ok().map(PathBuf::from);
		// Explicit command list: { "bash", "-c", "..." }
		let command: Option<Vec<String>> = tbl.get::<Vec<String>>("command").ok();

		if executable.is_none() && command.is_none() {
			return Err(mlua::Error::RuntimeError(format!(
				"Test '{}' must specify either 'binary' (a path) or 'command' (a list of strings).",
				name
			)));
		}

		let mut component = Component::test(&name, &target, executable, command);

		// Set package ID
		{
			let pkg = self.current_package.lock().unwrap();
			component = component.with_package(crate::graph::PackageId::new(pkg.as_str()));
		}

		if let Ok(vis_val) = tbl.get::<Value>("visibility") {
			component = component.with_visibility(parse_visibility(vis_val)?);
		}

		if let Ok(exec_cfg_str) = tbl.get::<String>("exec_cfg") {
			let exec_cfg = match exec_cfg_str.as_str() {
				"host" => crate::graph::ConfigTransition::Host,
				"exec" => crate::graph::ConfigTransition::Exec,
				"target" => crate::graph::ConfigTransition::Target,
				_ => return Err(mlua::Error::RuntimeError(format!("Invalid exec_cfg: {}", exec_cfg_str))),
			};
			component = component.with_exec_cfg(Some(exec_cfg));
		}

		// Propagate test-specific fields into the component_type
		use crate::graph::{TestKind, TestSize, ComponentType};
		if let ComponentType::Test {
			ref mut test_kind,
			ref mut args,
			ref mut data,
			ref mut env,
			ref mut timeout_secs,
			ref mut size,
			ref mut tags,
			..
		} = component.component_type
		{
			if let Ok(kind_str) = tbl.get::<String>("test_kind") {
				*test_kind = match kind_str.as_str() {
					"integration" => TestKind::Integration,
					"e2e" => TestKind::E2E,
					_ => TestKind::Unit,
				};
			}
			if let Ok(extra_args) = tbl.get::<Vec<String>>("args") {
				*args = extra_args;
			}
			if let Ok(data_files) = tbl.get::<Vec<String>>("data") {
				*data = data_files.into_iter().map(PathBuf::from).collect();
			}
			if let Ok(env_table) = tbl.get::<Table>("env") {
				for pair in env_table.pairs::<String, String>() {
					let (k, v) = pair?;
					env.insert(k, v);
				}
			}
			if let Ok(timeout) = tbl.get::<u64>("timeout") {
				*timeout_secs = timeout;
			}
			if let Ok(size_str) = tbl.get::<String>("size") {
				*size = match size_str.as_str() {
					"medium" => TestSize::Medium,
					"large" => TestSize::Large,
					_ => TestSize::Small,
				};
			}
			if let Ok(tag_list) = tbl.get::<Vec<String>>("tags") {
				*tags = tag_list;
			}
		}

		let mut g = self.graph.lock().unwrap();
		let comp_id = g
			.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		if let Ok(deps) = tbl.get::<Value>("deps") {
			match deps {
				Value::Table(deps_tbl) => {
					for pair in deps_tbl.pairs::<Value, Value>() {
						let (_key, val) = pair?;
						let (dep_ref, edge) = parse_dependency(val)?;
						g.add_dependency(comp_id, dep_ref, edge)
							.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
					}
				}
				_ => return Err(mlua::Error::RuntimeError("deps must be a list of strings or tables".into())),
			}
		}

		Ok(Value::Nil)
	}

	/// Get total component count
	pub fn component_count(&self) -> usize {
		let g = self.graph.lock().unwrap();
		g.component_count()
	}

	/// Get target count
	pub fn target_count(&self) -> usize {
		let g = self.graph.lock().unwrap();
		g.target_count()
	}

	/// Get components in topological order (dependency order)
	pub fn topological_order(&self) -> Result<Value> {
		let g = self.graph.lock().unwrap();
		let order = g.topological_order().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
		let lua = Lua::new();
		let result = lua.create_table()?;
		for (i, id) in order.iter().enumerate() {
			if let Some(comp) = g.get_component(*id) {
				result.set(i + 1, comp.name.clone())?;
			}
		}
		Ok(Value::Table(result))
	}

	/// Get components grouped by parallel execution batches
	pub fn execution_batches(&self) -> Result<Value> {
		let g = self.graph.lock().unwrap();
		let batches = g.execution_batches().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
		let lua = Lua::new();
		let result = lua.create_table()?;
		for (i, batch) in batches.iter().enumerate() {
			let batch_table = lua.create_table()?;
			for (j, id) in batch.iter().enumerate() {
				if let Some(comp) = g.get_component(*id) {
					batch_table.set(j + 1, comp.name.clone())?;
				}
			}
			result.set(i + 1, batch_table)?;
		}
		Ok(Value::Table(result))
	}

	/// Get direct dependencies of a component
	pub fn dependencies_of(&self, name: String, target: String) -> Vec<String> {
		let g = self.graph.lock().unwrap();
		if let Some(comp) = g.get_component_by_name(&name, &target) {
			g.dependencies_of(comp.id)
				.iter()
				.filter_map(|id| g.get_component(*id).map(|c| c.name.clone()))
				.collect()
		} else {
			vec![]
		}
	}

	/// Get include directories of a component
	pub fn get_component_includes(&self, name: String, target: String) -> Vec<String> {
		let g = self.graph.lock().unwrap();
		if let Some(comp) = g.get_component_by_name(&name, &target) {
			comp.include_dirs.iter().map(|p| p.to_string_lossy().to_string()).collect()
		} else {
			vec![]
		}
	}

	/// Get output files of a component
	pub fn get_component_outputs(&self, name: String, target: String) -> Vec<String> {
		let g = self.graph.lock().unwrap();
		if let Some(comp) = g.get_component_by_name(&name, &target) {
			comp.outputs.iter().map(|p| p.to_string_lossy().to_string()).collect()
		} else {
			vec![]
		}
	}

	/// Get all transitive dependencies of a component
	pub fn transitive_deps(&self, name: String, target: String) -> Vec<String> {
		let g = self.graph.lock().unwrap();
		if let Some(comp) = g.get_component_by_name(&name, &target) {
			g.transitive_dependencies(comp.id)
				.iter()
				.filter_map(|id| g.get_component(*id).map(|c| c.name.clone()))
				.collect()
		} else {
			vec![]
		}
	}

	/// Find dependency cycles in the graph, returns empty table if none found
	pub fn find_cycles(&self) -> Result<Value> {
		let g = self.graph.lock().unwrap();
		let cycles = g.find_cycles();
		let lua = Lua::new();
		let result = lua.create_table()?;
		if cycles.is_empty() {
			return Ok(Value::Table(result));
		}
		for (i, cycle) in cycles.iter().enumerate() {
			let cycle_table = lua.create_table()?;
			for (j, id) in cycle.iter().enumerate() {
				if let Some(comp) = g.get_component(*id) {
					cycle_table.set(j + 1, comp.name.clone())?;
				}
			}
			result.set(i + 1, cycle_table)?;
		}
		Ok(Value::Table(result))
	}

	/// Get components that directly depend on the given component (reverse deps)
	pub fn reverse_dependencies(&self, name: String, target: String) -> Vec<String> {
		let g = self.graph.lock().unwrap();
		if let Some(comp) = g.get_component_by_name(&name, &target) {
			g.reverse_dependencies(comp.id)
				.iter()
				.filter_map(|id| g.get_component(*id).map(|c| c.name.clone()))
				.collect()
		} else {
			vec![]
		}
	}

	/// Find components matching a glob pattern on their name
	pub fn components_matching(&self, pattern: String) -> Vec<String> {
		let g = self.graph.lock().unwrap();
		g.components_matching(&pattern)
			.iter()
			.filter_map(|id| g.get_component(*id).map(|c| c.name.clone()))
			.collect()
	}

	/// Render the build graph as a Graphviz DOT string
	pub fn output_dot(&self, edge_labels: Option<bool>) -> String {
		let g = self.graph.lock().unwrap();
		let opts = DotOptions {
			include_edge_labels: edge_labels.unwrap_or(true),
			color_by_type: true,
		};
		g.output_dot(&opts)
	}

	/// Check whether one component is allowed to depend on another.
	/// Throws a Lua error on violation.
	pub fn check_visibility(
		&self,
		from_name: String,
		from_target: String,
		to_name: String,
		to_target: String,
	) -> Result<()> {
		let g = self.graph.lock().unwrap();
		let from_id = g
			.get_component_by_name(&from_name, &from_target)
			.map(|c| c.id)
			.ok_or_else(|| mlua::Error::RuntimeError(format!("Component '{}:{}' not found", from_name, from_target)))?;
		let to_id = g
			.get_component_by_name(&to_name, &to_target)
			.map(|c| c.id)
			.ok_or_else(|| mlua::Error::RuntimeError(format!("Component '{}:{}' not found", to_name, to_target)))?;
		g.check_visibility(from_id, to_id).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
	}
}

fn infer_triple_from_name(name: &str) -> String {
	let name_lower = name.to_lowercase();
	if name_lower.contains("windows") {
		if name_lower.contains("x64") || name_lower.contains("x86_64") || name_lower.contains("amd64") {
			"x86_64-pc-windows-msvc".to_string()
		} else {
			"i686-pc-windows-msvc".to_string()
		}
	} else if name_lower.contains("linux") {
		if name_lower.contains("aarch64") || name_lower.contains("arm64") {
			"aarch64-unknown-linux-gnu".to_string()
		} else if name_lower.contains("arm") {
			"arm-unknown-linux-gnueabihf".to_string()
		} else if name_lower.contains("x64") || name_lower.contains("x86_64") {
			"x86_64-unknown-linux-gnu".to_string()
		} else {
			"x86_64-unknown-linux-gnu".to_string()
		}
	} else if name_lower.contains("macos") || name_lower.contains("darwin") {
		if name_lower.contains("aarch64") || name_lower.contains("arm64") {
			"aarch64-apple-darwin".to_string()
		} else {
			"x86_64-apple-darwin".to_string()
		}
	} else if name_lower.contains("freebsd") {
		"x86_64-unknown-freebsd".to_string()
	} else if name_lower.contains("wasm") {
		"wasm32-unknown-wasi".to_string()
	} else {
		let arch = if name_lower.contains("aarch64") || name_lower.contains("arm64") {
			"aarch64"
		} else if name_lower.contains("arm") {
			"arm"
		} else if name_lower.contains("x64") || name_lower.contains("x86_64") || name_lower.contains("amd64") {
			"x86_64"
		} else if name_lower.contains("i686") || name_lower.contains("i386") {
			"i686"
		} else {
			"x86_64"
		};

		let os = if name_lower.contains("windows") {
			"windows"
		} else if name_lower.contains("linux") {
			"linux"
		} else if name_lower.contains("macos") || name_lower.contains("darwin") {
			"darwin"
		} else if name_lower.contains("freebsd") {
			"freebsd"
		} else {
			"linux"
		};

		let abi = if name_lower.contains("musl") {
			"musl"
		} else if name_lower.contains("gnu") {
			"gnu"
		} else if os == "windows" {
			"msvc"
		} else {
			"gnu"
		};

		format!("{}-unknown-{}-{}", arch, os, abi)
	}
}

fn parse_visibility(val: Value) -> Result<crate::graph::Visibility> {
	match val {
		Value::Nil => Ok(crate::graph::Visibility::Public),
		Value::String(s) => {
			let s_str = s.to_str()?;
			match &*s_str {
				"public" => Ok(crate::graph::Visibility::Public),
				"package" => Ok(crate::graph::Visibility::Package),
				"private" => Ok(crate::graph::Visibility::Private),
				other => Err(mlua::Error::RuntimeError(format!("Invalid visibility level: {}", other))),
			}
		},
		Value::Table(t) => {
			let patterns: Vec<String> = t.sequence_values::<String>().collect::<Result<_>>()?;
			Ok(crate::graph::Visibility::Restricted(patterns))
		}
		_ => Err(mlua::Error::RuntimeError("Visibility must be a string or a list of patterns".into())),
	}
}

pub fn create_graph_table(lua: &Lua, project_root: PathBuf, graph: Arc<Mutex<BuildGraph>>) -> Result<Table> {
	let api = GraphApi::new(project_root, graph);
	api.create_graph_table(lua)
}

fn parse_dependency(val: Value) -> Result<(ComponentRef, DependencyEdge)> {
	match val {
		Value::String(s) => Ok((ComponentRef::new(s.to_str()?.to_string()), DependencyEdge::Hard)),
		Value::Table(t) => {
			let name: String = t.get("name").or_else(|_| t.get(1))?;
			let mut edge = DependencyEdge::Hard;
			if let Ok(trans_str) = t.get::<String>("transition") {
				let trans = match trans_str.as_str() {
					"host" => crate::graph::ConfigTransition::Host,
					"exec" => crate::graph::ConfigTransition::Exec,
					"target" => crate::graph::ConfigTransition::Target,
					_ => {
						return Err(mlua::Error::RuntimeError(format!(
							"Invalid transition: {}",
							trans_str
						)))
					}
				};
				edge = DependencyEdge::Transition(trans);
			}
			Ok((ComponentRef::new(name), edge))
		}
		_ => Err(mlua::Error::RuntimeError("Dependency must be a string or a table".into())),
	}
}
