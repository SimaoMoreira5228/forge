use crate::graph::{BuildGraph, Component, ComponentRef, DependencyEdge, Target};
use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct GraphApi {
	graph: Arc<Mutex<BuildGraph>>,
	project_root: PathBuf,
}

impl GraphApi {
	pub fn new(project_root: PathBuf) -> Self {
		Self {
			graph: Arc::new(Mutex::new(BuildGraph::new())),
			project_root,
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
		Self::new(project_root)
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

		if let Ok(defines) = tbl.get::<Vec<String>>("defines") {
			component = component.with_defines(defines.into_iter().map(|s| (s, None)).collect());
		}
		if let Ok(cflags) = tbl.get::<Vec<String>>("cflags") {
			component = component.with_compiler_flags(cflags);
		}

		let mut g = self.graph.lock().unwrap();
		let comp_id = g
			.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		if let Ok(deps) = tbl.get::<Vec<String>>("deps") {
			for dep in deps {
				let _ = g.add_dependency(comp_id, ComponentRef::new(&dep), DependencyEdge::Hard);
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

		let mut g = self.graph.lock().unwrap();
		let comp_id = g
			.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		if let Ok(deps) = tbl.get::<Vec<String>>("deps") {
			for dep in deps {
				let _ = g.add_dependency(comp_id, ComponentRef::new(&dep), DependencyEdge::Hard);
			}
		}

		Ok(Value::Nil)
	}

	/// Define a custom component with arbitrary command
	pub fn custom(&self, tbl: Table) -> Result<Value> {
		let name: String = tbl.get("name")?;
		let command: String = tbl.get("command")?;
		let args: Vec<String> = tbl.get("args").unwrap_or_default();
		let sources: Vec<String> = tbl.get("sources").or_else(|_| tbl.get("srcs")).unwrap_or_default();
		let outputs: Vec<String> = tbl.get("outputs").unwrap_or_default();

		let component = Component::custom(&name, "default", command, args)
			.with_sources(sources.iter().map(PathBuf::from).collect())
			.with_outputs(outputs.iter().map(PathBuf::from).collect());

		let mut g = self.graph.lock().unwrap();
		g.add_component(component)
			.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

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

pub fn create_graph_table(lua: &Lua, project_root: PathBuf) -> Result<Table> {
	let api = GraphApi::new(project_root);
	api.create_graph_table(lua)
}
