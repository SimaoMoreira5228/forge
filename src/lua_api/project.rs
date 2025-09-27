use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::path::Path;

#[derive(Clone)]
pub struct ProjectApi;

impl UserData for ProjectApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "project")]
impl ProjectApi {
	pub fn new() -> Self {
		Self
	}

	/// Resolve a path relative to the project root
	fn resolve(path: String, project_root: String) -> Result<String> {
		if std::path::Path::new(&path).is_absolute() {
			Ok(path)
		} else {
			Ok(Path::new(&project_root).join(path).to_string_lossy().to_string())
		}
	}
}

pub fn create_project_table(lua: &Lua, project_path: String) -> Result<Table> {
	let table = ProjectApi::create_project_table(lua)?;
	table.set("root", project_path)?;
	Ok(table)
}
