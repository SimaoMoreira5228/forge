use blake3::Hasher;
use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::{fs, path::Path};

#[derive(Clone)]
pub struct HashApi;

impl UserData for HashApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "hash")]
impl HashApi {
	pub fn new() -> Self {
		Self
	}

	/// Hash a file
	fn file(path: String) -> Result<String> {
		let mut hasher = Hasher::new();
		let content = fs::read(&path).map_err(mlua::Error::external)?;
		hasher.update(&content);
		Ok(hasher.finalize().to_hex().to_string())
	}

	/// Hash a string
	fn string(content: String) -> Result<String> {
		let mut hasher = Hasher::new();
		hasher.update(content.as_bytes());
		Ok(hasher.finalize().to_hex().to_string())
	}

	/// Hash multiple files into a single hash
	fn files(paths: Vec<String>) -> Result<String> {
		let mut hasher = Hasher::new();

		let mut sorted_paths = paths;
		sorted_paths.sort();

		for path in sorted_paths {
			let path_obj = Path::new(&path);

			hasher.update(path.as_bytes());
			hasher.update(&[0]);

			if path_obj.exists() && path_obj.is_file() {
				let content = fs::read(&path).map_err(mlua::Error::external)?;
				hasher.update(&content);
			}
			hasher.update(&[1]);
		}

		Ok(hasher.finalize().to_hex().to_string())
	}

	/// Verify file hash
	fn verify(path: String, expected_hash: String) -> Result<bool> {
		let mut hasher = Hasher::new();
		let content = fs::read(&path).map_err(mlua::Error::external)?;
		hasher.update(&content);
		let actual_hash = hasher.finalize().to_hex().to_string();

		Ok(actual_hash == expected_hash)
	}

	/// Hash bytes directly
	fn bytes(bytes: Vec<u8>) -> Result<String> {
		let mut hasher = Hasher::new();
		hasher.update(&bytes);
		Ok(hasher.finalize().to_hex().to_string())
	}
}

pub fn create_hash_table(lua: &Lua) -> Result<Table> {
	HashApi::create_hash_table(lua)
}
