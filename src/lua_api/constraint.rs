use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};

#[derive(Clone)]
pub struct ConstraintApi;

impl UserData for ConstraintApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "constraint")]
impl ConstraintApi {
	pub fn new() -> Self {
		Self
	}

	/// Create or reference a constraint by name
	fn get(name: String) -> Result<String> {
		Ok(name)
	}

	/// Add a new constraint type if not exists
	fn add(name: String) -> Result<()> {
		Ok(())
	}
}

pub fn create_constraint_table(lua: &Lua) -> Result<Table> {
	ConstraintApi::create_constraint_table(lua)
}
