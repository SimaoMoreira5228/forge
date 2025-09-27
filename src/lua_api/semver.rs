use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods, Value};
use semver::{Version, VersionReq};
use std::str::FromStr;

#[derive(Clone)]
pub struct SemverApi;

impl UserData for SemverApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "semver")]
impl SemverApi {
	pub fn new() -> Self {
		Self
	}

	/// Parse a version string and return a version table
	fn parse_version(lua: &Lua, version_str: String) -> Result<Table> {
		match Version::from_str(&version_str) {
			Ok(version) => lua_version_table(lua, &version),
			Err(e) => Err(mlua::Error::RuntimeError(format!("Invalid version '{}': {}", version_str, e))),
		}
	}

	/// Check if a version satisfies a requirement
	fn satisfies(version_str: String, req_str: String) -> Result<bool> {
		let version = Version::from_str(&version_str)
			.map_err(|e| mlua::Error::RuntimeError(format!("Invalid version '{}': {}", version_str, e)))?;
		let req = VersionReq::from_str(&req_str)
			.map_err(|e| mlua::Error::RuntimeError(format!("Invalid requirement '{}': {}", req_str, e)))?;

		Ok(req.matches(&version))
	}

	/// Compare two versions (-1, 0, 1)
	fn compare(version1_str: String, version2_str: String) -> Result<i32> {
		let version1 = Version::from_str(&version1_str)
			.map_err(|e| mlua::Error::RuntimeError(format!("Invalid version '{}': {}", version1_str, e)))?;
		let version2 = Version::from_str(&version2_str)
			.map_err(|e| mlua::Error::RuntimeError(format!("Invalid version '{}': {}", version2_str, e)))?;

		use std::cmp::Ordering;
		let result = match version1.cmp(&version2) {
			Ordering::Less => -1,
			Ordering::Equal => 0,
			Ordering::Greater => 1,
		};

		Ok(result)
	}

	/// Find the highest version in a list that satisfies a requirement
	fn find_best_match(lua: &Lua, versions: Vec<String>, req_str: String) -> Result<Value> {
		let req = VersionReq::from_str(&req_str)
			.map_err(|e| mlua::Error::RuntimeError(format!("Invalid requirement '{}': {}", req_str, e)))?;

		let mut matching_versions = Vec::new();
		for version_str in versions {
			match Version::from_str(&version_str) {
				Ok(version) => {
					if req.matches(&version) {
						matching_versions.push((version, version_str));
					}
				}
				Err(_) => continue,
			}
		}

		matching_versions.sort_by(|a, b| b.0.cmp(&a.0));

		match matching_versions.first() {
			Some((_, version_str)) => Ok(Value::String(lua.create_string(version_str)?)),
			None => Ok(Value::Nil),
		}
	}
}

fn lua_version_table(lua: &Lua, version: &Version) -> Result<Table> {
	let table = lua.create_table()?;
	table.set("major", version.major)?;
	table.set("minor", version.minor)?;
	table.set("patch", version.patch)?;
	table.set("pre", version.pre.to_string())?;
	table.set("build", version.build.to_string())?;
	table.set("to_string", version.to_string())?;
	Ok(table)
}

pub fn create_semver_table(lua: &Lua) -> Result<Table> {
	SemverApi::create_semver_table(lua)
}
