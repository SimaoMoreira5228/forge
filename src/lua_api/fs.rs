use anyhow::Result;
use forge_macros::lua_api;
use mlua::{Lua, Table, UserData, UserDataMethods};
use std::{
	fs,
	path::{Path, PathBuf},
	time::SystemTime,
};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Error, Debug)]
pub enum FsError {
	#[error("Path does not exist: {path}")]
	PathNotFound {
		path: String,
	},

	#[error("Invalid path: {path} - {reason}")]
	InvalidPath {
		path: String,
		reason: String,
	},

	#[error("Permission denied: {path}")]
	PermissionDenied {
		path: String,
	},

	#[error("Directory not empty: {path}")]
	DirectoryNotEmpty {
		path: String,
	},

	#[error("Invalid glob pattern: {pattern} - {reason}")]
	InvalidGlobPattern {
		pattern: String,
		reason: String,
	},

	#[error("Archive extraction failed: {archive} - {reason}")]
	ExtractionFailed {
		archive: String,
		reason: String,
	},
}

#[derive(Clone)]
pub struct FsApi;

impl UserData for FsApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

fn validate_path(path: &str) -> Result<PathBuf, FsError> {
	if path.is_empty() {
		return Err(FsError::InvalidPath {
			path: path.to_string(),
			reason: "Path cannot be empty".to_string(),
		});
	}

	let path_buf = PathBuf::from(path);
	if !path_buf.is_absolute() {
		return Err(FsError::InvalidPath {
			path: path.to_string(),
			reason: "Path must be absolute. Use forge.path.join() to build absolute paths".to_string(),
		});
	}

	Ok(path_buf)
}

#[lua_api(name = "fs")]
impl FsApi {
	pub fn new() -> Self {
		Self
	}

	/// Read file contents as string (path must be absolute)
	fn read(path: String) -> mlua::Result<String> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		if !path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: path.to_string_lossy().to_string(),
			}));
		}

		if !path.is_file() {
			return Err(mlua::Error::external(FsError::InvalidPath {
				path: path.to_string_lossy().to_string(),
				reason: "Path is not a file".to_string(),
			}));
		}

		fs::read_to_string(&path).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: path.to_string_lossy().to_string(),
			})
		})
	}

	/// Write string content to file (path must be absolute)
	fn write(path: String, content: String) -> mlua::Result<()> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).map_err(|_| {
				mlua::Error::external(FsError::PermissionDenied {
					path: parent.to_string_lossy().to_string(),
				})
			})?;
		}

		fs::write(&path, content).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: path.to_string_lossy().to_string(),
			})
		})
	}

	/// Create directory and all parent directories (path must be absolute)
	fn mkdir(path: String) -> mlua::Result<()> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		fs::create_dir_all(&path).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: path.to_string_lossy().to_string(),
			})
		})
	}

	/// Find files matching glob pattern (pattern must be absolute)
	fn glob(pattern: String) -> mlua::Result<Vec<String>> {
		let paths: Vec<String> = glob::glob(&pattern)
			.map_err(|e| {
				mlua::Error::external(FsError::InvalidGlobPattern {
					pattern: pattern.clone(),
					reason: format!("Invalid glob syntax: {}", e),
				})
			})?
			.filter_map(|res| res.ok())
			.map(|p| p.to_string_lossy().to_string())
			.collect();
		Ok(paths)
	}

	/// Check if file or directory exists (path must be absolute)
	fn exists(path: String) -> mlua::Result<bool> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;
		Ok(path.exists())
	}

	/// Get modification time as Unix timestamp (path must be absolute)
	fn mtime(path: String) -> mlua::Result<Option<u64>> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		if !path.exists() {
			return Ok(None);
		}

		let metadata = fs::metadata(&path).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: path.to_string_lossy().to_string(),
			})
		})?;

		if let Ok(modified) = metadata.modified()
			&& let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH)
		{
			return Ok(Some(duration.as_secs()));
		}

		Ok(None)
	}

	/// Copy file from source to destination (both paths must be absolute)
	fn copy(src: String, dest: String) -> mlua::Result<()> {
		let src_path = validate_path(&src).map_err(mlua::Error::external)?;
		let dest_path = validate_path(&dest).map_err(mlua::Error::external)?;

		if !src_path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: src_path.to_string_lossy().to_string(),
			}));
		}

		if let Some(parent) = dest_path.parent() {
			fs::create_dir_all(parent).map_err(|_| {
				mlua::Error::external(FsError::PermissionDenied {
					path: parent.to_string_lossy().to_string(),
				})
			})?;
		}

		fs::copy(&src_path, &dest_path).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: src_path.to_string_lossy().to_string(),
			})
		})?;
		Ok(())
	}

	/// Move/rename file from source to destination (both paths must be absolute)
	fn move_file(src: String, dest: String) -> mlua::Result<()> {
		let src_path = validate_path(&src).map_err(mlua::Error::external)?;
		let dest_path = validate_path(&dest).map_err(mlua::Error::external)?;

		if !src_path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: src_path.to_string_lossy().to_string(),
			}));
		}

		if let Some(parent) = dest_path.parent() {
			fs::create_dir_all(parent).map_err(|_| {
				mlua::Error::external(FsError::PermissionDenied {
					path: parent.to_string_lossy().to_string(),
				})
			})?;
		}

		fs::rename(&src_path, &dest_path).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: src_path.to_string_lossy().to_string(),
			})
		})?;
		Ok(())
	}

	/// Remove file or empty directory (path must be absolute)
	fn remove(path: String) -> mlua::Result<()> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		if !path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: path.to_string_lossy().to_string(),
			}));
		}

		if path.is_file() {
			fs::remove_file(&path).map_err(|_| {
				mlua::Error::external(FsError::PermissionDenied {
					path: path.to_string_lossy().to_string(),
				})
			})?;
		} else if path.is_dir() {
			fs::remove_dir(&path).map_err(|e| {
				if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
					mlua::Error::external(FsError::DirectoryNotEmpty {
						path: path.to_string_lossy().to_string(),
					})
				} else {
					mlua::Error::external(FsError::PermissionDenied {
						path: path.to_string_lossy().to_string(),
					})
				}
			})?;
		}
		Ok(())
	}

	/// Remove directory and all its contents (path must be absolute)
	fn remove_dir(path: String) -> mlua::Result<()> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		if !path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: path.to_string_lossy().to_string(),
			}));
		}

		fs::remove_dir_all(&path).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: path.to_string_lossy().to_string(),
			})
		})
	}

	/// Check if path is a file (path must be absolute)
	fn is_file(path: String) -> mlua::Result<bool> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;
		Ok(path.is_file())
	}

	/// Check if path is a directory (path must be absolute)
	fn is_dir(path: String) -> mlua::Result<bool> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;
		Ok(path.is_dir())
	}

	/// Walk directory tree (path must be absolute)
	fn walk(path: String, options: Option<Table>) -> mlua::Result<Vec<String>> {
		let path = validate_path(&path).map_err(mlua::Error::external)?;

		if !path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: path.to_string_lossy().to_string(),
			}));
		}

		if !path.is_dir() {
			return Err(mlua::Error::external(FsError::InvalidPath {
				path: path.to_string_lossy().to_string(),
				reason: "Path is not a directory".to_string(),
			}));
		}

		let recursive = options
			.as_ref()
			.and_then(|opts| opts.get::<Option<bool>>("recursive").ok().flatten())
			.unwrap_or(true);

		let mut walker = WalkDir::new(&path);
		if !recursive {
			walker = walker.max_depth(1);
		}

		let paths: Vec<String> = walker
			.into_iter()
			.filter_map(|entry| entry.ok())
			.map(|entry| entry.path().to_string_lossy().to_string())
			.collect();

		Ok(paths)
	}

	/// Get system temporary directory
	fn temp_dir() -> mlua::Result<String> {
		Ok(std::env::temp_dir().to_string_lossy().to_string())
	}

	/// Create temporary file with optional prefix
	fn temp_file(prefix: Option<String>) -> mlua::Result<String> {
		let prefix = prefix.unwrap_or_else(|| "forge_temp".to_string());
		let temp_dir = std::env::temp_dir();
		let temp_file = temp_dir.join(format!("{}_{}", prefix, uuid::Uuid::new_v4()));

		std::fs::File::create(&temp_file).map_err(|_| {
			mlua::Error::external(FsError::PermissionDenied {
				path: temp_file.to_string_lossy().to_string(),
			})
		})?;

		Ok(temp_file.to_string_lossy().to_string())
	}

	/// Extract archive to destination (both paths must be absolute)
	fn extract(options: Table) -> mlua::Result<String> {
		let archive_path: String = options.get("archive")?;
		let dest_path: String = options.get("dest")?;

		let archive_path = validate_path(&archive_path).map_err(mlua::Error::external)?;
		let dest_path = validate_path(&dest_path).map_err(mlua::Error::external)?;

		if !archive_path.exists() {
			return Err(mlua::Error::external(FsError::PathNotFound {
				path: archive_path.to_string_lossy().to_string(),
			}));
		}

		extract_archive(&archive_path, &dest_path).map_err(mlua::Error::external)?;

		Ok(dest_path.to_string_lossy().to_string())
	}
}

pub fn extract_archive(archive_path: &Path, dest_path: &Path) -> Result<(), FsError> {
	use flate2::read::GzDecoder;
	use std::fs::File;
	use tar::Archive;
	use zip::ZipArchive;

	std::fs::create_dir_all(dest_path).map_err(|_| FsError::PermissionDenied {
		path: dest_path.to_string_lossy().to_string(),
	})?;

	let file = File::open(archive_path).map_err(|_| FsError::PathNotFound {
		path: archive_path.to_string_lossy().to_string(),
	})?;

	let extension = archive_path.extension().and_then(|s| s.to_str());

	match extension {
		Some("zip") => {
			let mut archive = ZipArchive::new(file).map_err(|e| FsError::ExtractionFailed {
				archive: archive_path.to_string_lossy().to_string(),
				reason: format!("Invalid zip file: {}", e),
			})?;
			archive.extract(dest_path).map_err(|e| FsError::ExtractionFailed {
				archive: archive_path.to_string_lossy().to_string(),
				reason: format!("Extraction error: {}", e),
			})?;
		}
		Some("gz")
			if archive_path
				.file_name()
				.and_then(|s| s.to_str())
				.is_some_and(|s| s.contains(".tar.")) =>
		{
			let tar = GzDecoder::new(file);
			let mut archive = Archive::new(tar);
			archive.unpack(dest_path).map_err(|e| FsError::ExtractionFailed {
				archive: archive_path.to_string_lossy().to_string(),
				reason: format!("Extraction error: {}", e),
			})?;
		}
		Some("tar") => {
			let mut archive = Archive::new(file);
			archive.unpack(dest_path).map_err(|e| FsError::ExtractionFailed {
				archive: archive_path.to_string_lossy().to_string(),
				reason: format!("Extraction error: {}", e),
			})?;
		}
		Some("crate") => {
			let tar = GzDecoder::new(file);
			let mut archive = Archive::new(tar);

			for entry in archive.entries().map_err(|e| FsError::ExtractionFailed {
				archive: archive_path.to_string_lossy().to_string(),
				reason: format!("Invalid crate file: {}", e),
			})? {
				let mut entry = entry.map_err(|e| FsError::ExtractionFailed {
					archive: archive_path.to_string_lossy().to_string(),
					reason: format!("Extraction error: {}", e),
				})?;
				let path = entry.path().map_err(|e| FsError::ExtractionFailed {
					archive: archive_path.to_string_lossy().to_string(),
					reason: format!("Invalid path in archive: {}", e),
				})?;
				let components: Vec<_> = path.components().collect();

				if components.len() > 1 {
					let new_path: PathBuf = components[1..].iter().collect();
					let dest_file_path = dest_path.join(&new_path);

					if let Some(parent) = dest_file_path.parent() {
						std::fs::create_dir_all(parent).map_err(|_| FsError::PermissionDenied {
							path: parent.to_string_lossy().to_string(),
						})?;
					}

					entry.unpack(&dest_file_path).map_err(|e| FsError::ExtractionFailed {
						archive: archive_path.to_string_lossy().to_string(),
						reason: format!("Extraction error: {}", e),
					})?;
				}
			}
		}
		_ => {
			return Err(FsError::ExtractionFailed {
				archive: archive_path.to_string_lossy().to_string(),
				reason: format!("Unsupported archive format: {:?}", archive_path),
			});
		}
	}

	Ok(())
}

pub fn create_fs_table(lua: &Lua) -> mlua::Result<Table> {
	FsApi::create_fs_table(lua)
}
