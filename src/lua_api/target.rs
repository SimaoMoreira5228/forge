use crate::target::{TargetDefinition, TargetResolver};
use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};

#[derive(Clone)]
pub struct TargetApi {
	resolver: TargetResolver,
}

impl TargetApi {
	pub fn new() -> Self {
		Self {
			resolver: TargetResolver::new(),
		}
	}
}

impl Default for TargetApi {
	fn default() -> Self {
		Self::new()
	}
}

impl UserData for TargetApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "target")]
impl TargetApi {
	pub fn create() -> Self {
		Self::new()
	}

	/// List all available predefined targets
	pub fn list(&self, lua: &Lua) -> Result<Table> {
		let result = lua.create_table()?;
		let targets = self.resolver.all_targets();
		for (i, target) in targets.enumerate() {
			let t = lua.create_table()?;
			t.set("name", target.name.clone())?;
			t.set("triple", target.canonical_name.clone())?;
			t.set("arch", target.arch.clone())?;
			t.set("os", target.os.clone())?;
			t.set("abi", target.abi.clone())?;
			result.set(i + 1, t)?;
		}
		Ok(result)
	}

	/// Resolve a target by name, returns table with target info or nil if not found
	pub fn resolve(&self, name: String, lua: &Lua) -> Result<Table> {
		match self.resolver.resolve(&name) {
			Some(t) => {
				let result = lua.create_table()?;
				result.set("name", t.name.clone())?;
				result.set("triple", t.canonical_name.clone())?;
				result.set("arch", t.arch.clone())?;
				result.set("os", t.os.clone())?;
				result.set("abi", t.abi.clone())?;
				Ok(result)
			}
			None => Ok(lua.create_table()?),
		}
	}

	/// Get canonical triple string for a target name
	pub fn canonical_triple(&self, name: String) -> Option<String> {
		self.resolver.get_canonical_triple(&name)
	}

	/// Get the host target definition
	pub fn host(&self, lua: &Lua) -> Result<Table> {
		let host = TargetDefinition::host();
		let result = lua.create_table()?;
		result.set("name", host.name.clone())?;
		result.set("triple", host.canonical_name.clone())?;
		result.set("arch", host.arch.clone())?;
		result.set("os", host.os.clone())?;
		result.set("abi", host.abi.clone())?;
		Ok(result)
	}

	/// Get output directory for a target
	pub fn output_dir(&self, args: Table) -> Result<String> {
		let name: String = args.get("target")?;
		let project_root: String = args.get("project_root").unwrap_or_else(|_| ".".to_string());

		let root = std::path::PathBuf::from(&project_root);
		Ok(self
			.resolver
			.get_output_directory(&name, &root)
			.unwrap_or_default()
			.to_string_lossy()
			.to_string())
	}
}

pub fn create_target_table(lua: &Lua) -> Result<Table> {
	let api = TargetApi::new();
	api.create_target_table(lua)
}
