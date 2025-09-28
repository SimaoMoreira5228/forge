use std::path::PathBuf;

use crate::project::{Project, Rule};
use crate::{error::ForgeError, lua_api};
use mlua::{Lua, LuaSerdeExt, Table};

pub fn setup_lua_environment(lua: &Lua, project: &Project) -> Result<(), ForgeError> {
	let globals = lua.globals();
	let forge_table = lua.create_table()?;

	let project_path = project.path.to_string_lossy().to_string();
	forge_table.set("config", lua.to_value(&project.config)?)?;

	forge_table.set("fs", lua_api::fs::create_fs_table(lua)?)?;
	forge_table.set("http", lua_api::http::create_http_table(lua)?)?;
	forge_table.set("parse", lua_api::parse::create_parse_table(lua)?)?;
	forge_table.set("exec", lua_api::exec::create_exec_table(lua)?)?;
	forge_table.set("semver", lua_api::semver::create_semver_table(lua)?)?;
	forge_table.set("platform", lua_api::platform::create_platform_table(lua)?)?;
	forge_table.set("path", lua_api::path::create_path_table(lua)?)?;
	forge_table.set("string", lua_api::string::create_string_table(lua)?)?;
	forge_table.set("hash", lua_api::hash::create_hash_table(lua)?)?;
	forge_table.set("time", lua_api::time::create_time_table(lua)?)?;
	forge_table.set("log", lua_api::log::create_log_table(lua)?)?;
	forge_table.set("table", lua_api::table::create_table_table(lua)?)?;
	forge_table.set("project", lua_api::project::create_project_table(lua, project_path.clone())?)?;

	let prelude_path = project.path.join("prelude");

	let build_graph = project.build_graph.clone();
	let output_map = project.output_map.clone();
	let project_path_for_rule = project.path.clone();
	let project_path_for_loader = project.path.clone();

	let rule_fn = lua.create_function(move |_, tbl: Table| {
		let name: String = tbl.get("name")?;
		let command: String = tbl.get("command")?;
		let args: Vec<String> = tbl.get("args").unwrap_or_default();
		let inputs: Vec<String> = tbl.get("inputs").unwrap_or_default();
		let outputs: Vec<String> = tbl.get("outputs").unwrap_or_default();
		let dependencies: Vec<String> = tbl.get("dependencies").unwrap_or_default();
		let env: Option<Table> = tbl.get("env")?;
		let workdir: Option<String> = tbl.get("workdir")?;

		let env_map: std::collections::HashMap<String, String> = if let Some(env_table) = env {
			env_table
				.pairs::<String, String>()
				.map(|pair| pair.map_err(mlua::Error::external))
				.collect::<Result<_, _>>()?
		} else {
			std::collections::HashMap::new()
		};

		let rule_workdir = if let Some(wd) = workdir {
			PathBuf::from(wd)
		} else {
			project_path_for_rule.clone()
		};

		let rule = Rule {
			name: name.clone(),
			command,
			args,
			env: env_map,
			inputs,
			outputs: outputs.clone(),
			dependencies,
			workdir: rule_workdir,
		};

		for output in &outputs {
			output_map.insert(output.clone(), name.clone());
		}

		build_graph.insert(name, rule);
		Ok(())
	})?;

	forge_table.set("rule", rule_fn)?;

	let sleep_fn = lua.create_function(|_, duration: f64| {
		let duration = std::time::Duration::from_secs_f64(duration);
		std::thread::sleep(duration);
		Ok(())
	})?;
	forge_table.set("sleep", sleep_fn)?;

	let package: Table = globals.get("package")?;
	let prelude_loader = lua.create_function(move |lua, module_name: String| {
		let mut path_to_try = PathBuf::new();
		let mut module_display_name = module_name.clone();

		if let Some(stripped) = module_name.strip_prefix("@prelude/") {
			path_to_try = prelude_path.join(stripped);
			module_display_name = format!("@prelude/{}", stripped);
		} else if !module_name.starts_with('@') {
			path_to_try = project_path_for_loader.join(&module_name);
		}

		if path_to_try.exists() {
			let content = std::fs::read_to_string(&path_to_try).map_err(mlua::Error::external)?;
			let chunk = lua.load(&content).set_name(&module_display_name);
			return Ok(Some(chunk.into_function()?));
		}

		Ok(None)
	})?;

	let searchers: Table = package.get("searchers").or_else(|_| package.get("loaders"))?;
	searchers.set(2, prelude_loader)?;

	globals.set("forge", forge_table)?;
	Ok(())
}

pub fn generate_types_lua() -> String {
	let mut types = String::new();

	types.push_str("-- Generated Lua type definitions for Forge APIs\n");
	types.push_str("-- This file provides type hints for Lua language servers\n\n");

	types.push_str(lua_api::fs::FsApi::fs_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::http::HttpApi::http_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::parse::ParseApi::parse_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::exec::ExecApi::exec_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::semver::SemverApi::semver_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::platform::PlatformApi::platform_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::path::PathApi::path_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::string::StringApi::string_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::hash::HashApi::hash_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::time::TimeApi::time_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::log::LogApi::log_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::table::TableApi::table_lua_type_definitions());
	types.push('\n');
	types.push_str(lua_api::project::ProjectApi::project_lua_type_definitions());
	types.push('\n');

	types.push_str("---@class Forge\n");
	types.push_str("---@field config table Configuration table\n");
	types.push_str("---@field fs Fs File system operations (all paths must be absolute)\n");
	types.push_str("---@field http Http HTTP operations\n");
	types.push_str("---@field parse Parse Parsing operations\n");
	types.push_str("---@field exec Exec Command execution operations\n");
	types.push_str("---@field semver Semver Semantic versioning operations\n");
	types.push_str("---@field platform Platform Platform detection operations\n");
	types.push_str("---@field path Path Path manipulation operations\n");
	types.push_str("---@field string String String manipulation operations\n");
	types.push_str("---@field hash Hash Hashing operations\n");
	types.push_str("---@field time Time Time operations\n");
	types.push_str("---@field log Log Logging operations\n");
	types.push_str("---@field table Table Table operations\n");
	types.push_str("---@field project Project Project context and utilities\n");
	types.push_str("---@field rule fun(rule: table): nil Add a build rule\n");
	types.push_str("---@field sleep fun(seconds: number): nil Sleep for specified seconds\n");
	types.push('\n');

	types.push_str("---@class Project\n");
	types.push_str("---@field root string Absolute path to project root\n");
	types.push_str(
		"---@field resolve fun(path: string): string Convert relative path to absolute (relative to project root)\n",
	);
	types.push_str("\n---@type Forge\n");
	types.push_str("forge = nil\n");

	types
}
