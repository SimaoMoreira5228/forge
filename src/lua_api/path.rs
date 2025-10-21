use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct PathApi;

impl UserData for PathApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "path")]
impl PathApi {
	pub fn new() -> Self {
		Self
	}

	/// Join path components
	fn join(components: Vec<String>) -> Result<String> {
		let path = components.iter().fold(PathBuf::new(), |acc, component| acc.join(component));
		Ok(path.to_string_lossy().to_string())
	}

	/// Get directory name (parent directory)
	fn dirname(path: String) -> Result<String> {
		let path = Path::new(&path);
		Ok(path
			.parent()
			.map(|p| p.to_string_lossy().to_string())
			.unwrap_or_else(|| ".".to_string()))
	}

	/// Get base name (file name with extension)
	fn basename(path: String) -> Result<String> {
		let path = Path::new(&path);
		Ok(path
			.file_name()
			.map(|name| name.to_string_lossy().to_string())
			.unwrap_or_else(|| path.to_string_lossy().to_string()))
	}

	/// Get file extension
	fn extension(path: String) -> Result<String> {
		let path = Path::new(&path);
		Ok(path
			.extension()
			.map(|ext| format!(".{}", ext.to_string_lossy()))
			.unwrap_or_default())
	}

	/// Get file stem (name without extension)
	fn stem(path: String) -> Result<String> {
		let path = Path::new(&path);
		Ok(path
			.file_stem()
			.map(|stem| stem.to_string_lossy().to_string())
			.unwrap_or_default())
	}

	/// Check if path is absolute
	fn is_absolute(path: String) -> Result<bool> {
		let path = Path::new(&path);
		Ok(path.is_absolute())
	}

	/// Check if path is relative
	fn is_relative(path: String) -> Result<bool> {
		let path = Path::new(&path);
		Ok(path.is_relative())
	}

	/// Canonicalize path (resolve to absolute path)
	fn canonicalize(path: String) -> Result<String> {
		let path = Path::new(&path);
		path.canonicalize()
			.map(|p| p.to_string_lossy().to_string())
			.map_err(mlua::Error::external)
	}

	/// Get absolute path without resolving symlinks
	fn absolute(path: String) -> Result<String> {
		let path = Path::new(&path);
		if path.is_absolute() {
			Ok(path.to_string_lossy().to_string())
		} else {
			let cwd = std::env::current_dir().map_err(mlua::Error::external)?;
			Ok(cwd.join(path).to_string_lossy().to_string())
		}
	}

	/// Normalize path (remove . and .. components)
	fn normalize(path: String) -> Result<String> {
		let path = Path::new(&path);
		let mut components = Vec::new();

		for component in path.components() {
			match component {
				std::path::Component::CurDir => {}
				std::path::Component::ParentDir => {
					if !components.is_empty() {
						components.pop();
					}
				}
				_ => {
					components.push(component.as_os_str());
				}
			}
		}

		let normalized: PathBuf = components.iter().collect();
		Ok(normalized.to_string_lossy().to_string())
	}

	/// Get home directory
	fn home() -> Result<String> {
		dirs::home_dir()
			.map(|p| p.to_string_lossy().to_string())
			.ok_or_else(|| mlua::Error::RuntimeError("Could not find home directory".into()))
	}
}

pub fn create_path_table(lua: &Lua) -> Result<Table> {
	PathApi::create_path_table(lua)
}
