use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods, Value};

#[derive(Clone)]
pub struct TableApi;

impl UserData for TableApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "table")]
impl TableApi {
	pub fn new() -> Self {
		Self
	}

	/// Get the length of a table (counts all key-value pairs)
	/// This works for both array-like and map-like tables
	fn length(tbl: Table) -> Result<usize> {
		let mut count = 0;
		for pair in tbl.pairs::<Value, Value>() {
			pair?;
			count += 1;
		}
		Ok(count)
	}

	/// Check if a table is empty
	fn is_empty(tbl: Table) -> Result<bool> {
		if let Some(pair) = tbl.pairs::<Value, Value>().next() {
			pair?;
			return Ok(false);
		}
		Ok(true)
	}

	/// Get all keys from a table
	fn keys(tbl: Table) -> Result<Vec<Value>> {
		let mut keys = Vec::new();
		for pair in tbl.pairs::<Value, Value>() {
			let (key, _) = pair?;
			keys.push(key);
		}
		Ok(keys)
	}

	/// Get all values from a table
	fn values(tbl: Table) -> Result<Vec<Value>> {
		let mut values = Vec::new();
		for pair in tbl.pairs::<Value, Value>() {
			let (_, value) = pair?;
			values.push(value);
		}
		Ok(values)
	}

	/// Check if a table contains a specific key
	fn contains_key(tbl: Table, key: Value) -> Result<bool> {
		let value: Value = tbl.get(key)?;
		Ok(!matches!(value, Value::Nil))
	}

	/// Merge two tables (second table overwrites values from first on key conflicts)
	fn merge(lua: &Lua, tbl1: Table, tbl2: Table) -> Result<Table> {
		let result = lua.create_table()?;

		for pair in tbl1.pairs::<Value, Value>() {
			let (key, value) = pair?;
			result.set(key, value)?;
		}

		for pair in tbl2.pairs::<Value, Value>() {
			let (key, value) = pair?;
			result.set(key, value)?;
		}

		Ok(result)
	}
}

pub fn create_table_table(lua: &Lua) -> Result<Table> {
	TableApi::create_table_table(lua)
}
