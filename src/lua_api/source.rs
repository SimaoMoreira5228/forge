use crate::source::SourceResolver;
use forge_macros::lua_api;
use mlua::{AnyUserData, Lua, Result, Table, UserData, UserDataMethods};
use std::path::PathBuf;

#[derive(Clone)]
pub struct SourceApi {
	project_root: PathBuf,
}

impl SourceApi {
	pub fn new(project_root: PathBuf) -> Self {
		Self { project_root }
	}
}

impl UserData for SourceApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "source")]
impl SourceApi {
	pub fn create(project_root: PathBuf) -> Self {
		Self::new(project_root)
	}

	/// Find files matching glob pattern
	pub fn glob(&self, pattern: String, lua: &Lua) -> Result<Table> {
		let resolver = SourceResolver::new(self.project_root.clone());
		let matches = resolver.glob(&pattern);

		let result = lua.create_table()?;
		for (i, path) in matches.iter().enumerate() {
			result.set(i + 1, path.to_string_lossy().to_string())?;
		}
		Ok(result)
	}

	/// Find files recursively matching pattern
	pub fn glob_recursive(&self, args: Table, lua: &Lua) -> Result<Table> {
		let pattern: String = args.get("pattern")?;
		let max_depth: Option<usize> = args.get("max_depth").ok();

		let resolver = SourceResolver::new(self.project_root.clone());
		let matches = resolver.glob_recursive(&pattern, max_depth);

		let result = lua.create_table()?;
		for (i, path) in matches.iter().enumerate() {
			result.set(i + 1, path.to_string_lossy().to_string())?;
		}
		Ok(result)
	}

	/// Resolve source file patterns to absolute paths
	pub fn resolve(&self, args: Table, lua: &Lua) -> Result<Table> {
		let patterns: Vec<String> = args.get("patterns")?;

		let resolver = SourceResolver::new(self.project_root.clone());
		let matches = resolver.resolve_sources(&patterns);

		let result = lua.create_table()?;
		for (i, path) in matches.iter().enumerate() {
			result.set(i + 1, path.to_string_lossy().to_string())?;
		}
		Ok(result)
	}

	/// Resolve include directories
	pub fn includes(&self, args: Table, lua: &Lua) -> Result<Table> {
		let dirs: Vec<String> = args.get("dirs")?;

		let resolver = SourceResolver::new(self.project_root.clone());
		let matches = resolver.resolve_includes(&dirs);

		let result = lua.create_table()?;
		for (i, path) in matches.iter().enumerate() {
			result.set(i + 1, path.to_string_lossy().to_string())?;
		}
		Ok(result)
	}

	/// Convert path to absolute
	pub fn absolute(&self, path: String) -> String {
		let resolver = SourceResolver::new(self.project_root.clone());
		resolver.to_absolute(&path).to_string_lossy().to_string()
	}

	/// Get path relative to project root
	pub fn relative(&self, path: String) -> String {
		let resolver = SourceResolver::new(self.project_root.clone());
		resolver.relative(&path).to_string_lossy().to_string()
	}

	/// Get project root path
	pub fn root(&self) -> String {
		self.project_root.to_string_lossy().to_string()
	}
}

pub fn create_source_table(lua: &Lua, project_root: PathBuf) -> Result<AnyUserData> {
	let api = SourceApi::new(project_root);
	lua.create_userdata(api)
}
