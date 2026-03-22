use forge_macros::lua_api;
use mlua::{Lua, LuaSerdeExt, Result, Table, UserData, UserDataMethods, Value};

#[derive(Clone)]
pub struct JsonApi;

impl UserData for JsonApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "json")]
impl JsonApi {
	pub fn new() -> Self {
		Self
	}

	fn encode(_lua: &Lua, value: Value) -> Result<String> {
		let json_value = sonic_rs::to_string(&value).map_err(mlua::Error::external)?;
		Ok(json_value)
	}

	fn decode(lua: &Lua, json_str: String) -> Result<Value> {
		let value: sonic_rs::Value = sonic_rs::from_str(&json_str).map_err(mlua::Error::external)?;
		lua.to_value(&value)
	}
}

pub fn create_json_table(lua: &Lua) -> Result<Table> {
	JsonApi::create_json_table(lua)
}
