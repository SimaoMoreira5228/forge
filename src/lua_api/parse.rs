use forge_macros::lua_api;
use mlua::{Lua, LuaSerdeExt, Result, Table, UserData, UserDataMethods, Value};

#[derive(Clone)]
pub struct ParseApi;

impl UserData for ParseApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "parse")]
impl ParseApi {
	pub fn new() -> Self {
		Self
	}

	/// Parse JSON string
	fn json(lua: &Lua, json_str: String) -> Result<Value> {
		let value: serde_json::Value = serde_json::from_str(&json_str).map_err(mlua::Error::external)?;
		lua.to_value(&value)
	}

	/// Parse TOML string
	fn toml(lua: &Lua, toml_str: String) -> Result<Value> {
		let value: toml::Value = toml::from_str(&toml_str).map_err(mlua::Error::external)?;
		lua.to_value(&value)
	}
}

pub fn create_parse_table(lua: &Lua) -> Result<Table> {
	ParseApi::create_parse_table(lua)
}
