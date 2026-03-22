use forge_macros::lua_api;
use mlua::{Lua, LuaSerdeExt, Result, Table, UserData, UserDataMethods, Value};

#[derive(Clone)]
pub struct TomlApi;

impl UserData for TomlApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "toml")]
impl TomlApi {
	pub fn new() -> Self {
		Self
	}

	fn encode(_lua: &Lua, value: Value) -> Result<String> {
		let toml_value = toml::Value::try_from(value).map_err(mlua::Error::external)?;
		let s = toml_value.to_string();
		Ok(s)
	}

	fn decode(lua: &Lua, toml_str: String) -> Result<Value> {
		let value: toml::Value = toml::from_str(&toml_str).map_err(mlua::Error::external)?;
		lua.to_value(&value)
	}
}

pub fn create_toml_table(lua: &Lua) -> Result<Table> {
	TomlApi::create_toml_table(lua)
}
