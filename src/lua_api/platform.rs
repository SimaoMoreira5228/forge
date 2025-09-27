use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::env;

#[derive(Clone)]
pub struct PlatformApi;

impl UserData for PlatformApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "platform")]
impl PlatformApi {
	pub fn new() -> Self {
		Self
	}

	/// Get operating system
	fn os() -> Result<&'static str> {
		Ok(if cfg!(target_os = "windows") {
			"windows"
		} else if cfg!(target_os = "macos") {
			"macos"
		} else if cfg!(target_os = "linux") {
			"linux"
		} else if cfg!(target_os = "freebsd") {
			"freebsd"
		} else {
			"unknown"
		})
	}

	/// Get architecture
	fn arch() -> Result<&'static str> {
		Ok(if cfg!(target_arch = "x86_64") {
			"x86_64"
		} else if cfg!(target_arch = "x86") {
			"x86"
		} else if cfg!(target_arch = "aarch64") {
			"aarch64"
		} else if cfg!(target_arch = "arm") {
			"arm"
		} else {
			"unknown"
		})
	}

	/// Check if running on Windows
	fn is_windows() -> Result<bool> {
		Ok(cfg!(target_os = "windows"))
	}

	/// Check if running on macOS
	fn is_macos() -> Result<bool> {
		Ok(cfg!(target_os = "macos"))
	}

	/// Check if running on Linux
	fn is_linux() -> Result<bool> {
		Ok(cfg!(target_os = "linux"))
	}

	/// Get path separator
	fn path_separator() -> Result<String> {
		Ok(std::path::MAIN_SEPARATOR.to_string())
	}

	/// Get executable extension
	fn exe_extension() -> Result<&'static str> {
		Ok(if cfg!(target_os = "windows") { ".exe" } else { "" })
	}

	/// Get current working directory
	fn cwd() -> Result<String> {
		env::current_dir()
			.map(|p| p.to_string_lossy().to_string())
			.map_err(mlua::Error::external)
	}
}

pub fn create_platform_table(lua: &Lua) -> Result<Table> {
	PlatformApi::create_platform_table(lua)
}
