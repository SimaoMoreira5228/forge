use crate::error::ForgeError;
use crate::lua_api::fs::extract_archive;
use blake3::Hasher as Blake3Hasher;
use forge_macros::lua_api;
use mlua::{FromLua, Lua, LuaSerdeExt, Result, Table, UserData, UserDataMethods, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::{
	fs::{self},
	path::PathBuf,
};

#[derive(Debug, Deserialize, Serialize)]
pub struct HttpGetRequest {
	pub url: String,
	pub timeout: Option<u64>,
	pub follow_redirects: Option<bool>,
	pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HttpPostRequest {
	pub url: String,
	pub timeout: Option<u64>,
	pub follow_redirects: Option<bool>,
	pub headers: Option<HashMap<String, String>>,
	pub body: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HttpDownloadRequest {
	pub url: String,
	pub cache_key: Option<String>,
	pub blake3: Option<String>,
	pub sha256: Option<String>,
	pub extract: Option<bool>,
	pub extract_dir: Option<String>,
}

impl FromLua for HttpGetRequest {
	fn from_lua(value: Value, lua: &Lua) -> Result<Self> {
		lua.from_value(value)
	}
}

impl FromLua for HttpPostRequest {
	fn from_lua(value: Value, lua: &Lua) -> Result<Self> {
		lua.from_value(value)
	}
}

impl FromLua for HttpDownloadRequest {
	fn from_lua(value: Value, lua: &Lua) -> Result<Self> {
		lua.from_value(value)
	}
}

#[derive(Clone)]
pub struct HttpApi;

impl UserData for HttpApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "http")]
impl HttpApi {
	pub fn new() -> Self {
		Self
	}

	/// Perform HTTP GET request
	fn get(lua: &Lua, request: HttpGetRequest) -> Result<Value> {
		let agent_config = ureq::Agent::config_builder()
			.timeout_global(request.timeout.map(std::time::Duration::from_secs))
			.max_redirects(if request.follow_redirects.unwrap_or(true) { 10 } else { 0 })
			.build();

		let agent: ureq::Agent = agent_config.into();
		let mut req = agent.get(&request.url);
		req = req.header("User-Agent", format!("forge/{}", env!("CARGO_PKG_VERSION")));

		if let Some(headers) = request.headers {
			for (key, value) in headers {
				req = req.header(key, value);
			}
		}

		let mut response = req.call().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

		let table = lua.create_table()?;
		table.set("status", response.status().as_u16())?;

		if response.status() == 200 {
			let body = response.body_mut().read_to_string().map_err(mlua::Error::external)?;
			table.set("body", body)?;
		}

		Ok(Value::Table(table))
	}

	/// Perform HTTP POST request
	fn post(lua: &Lua, request: HttpPostRequest) -> Result<Value> {
		let agent_config = ureq::Agent::config_builder()
			.timeout_global(request.timeout.map(std::time::Duration::from_secs))
			.max_redirects(if request.follow_redirects.unwrap_or(true) { 10 } else { 0 })
			.build();

		let agent: ureq::Agent = agent_config.into();
		let mut req = agent.post(&request.url);

		if let Some(headers) = request.headers {
			for (key, value) in headers {
				req = req.header(key, value);
			}
		}

		req = req.header("User-Agent", format!("forge/{}", env!("CARGO_PKG_VERSION")));

		let response = req
			.send(request.body.unwrap_or("".into()))
			.map_err(|e| mlua::Error::RuntimeError(format!("HTTP request failed: {}", e)))?;

		let status = response.status();
		let result = lua.create_table()?;
		result.set("status", status.as_u16())?;

		let response_headers = lua.create_table()?;
		for (name, value) in response.headers() {
			response_headers.set(name.as_str(), value.to_str().map_err(mlua::Error::external)?)?;
		}
		result.set("headers", response_headers)?;

		Ok(Value::Table(result))
	}

	/// Download and cache a file
	fn download(request: HttpDownloadRequest) -> Result<String> {
		let cache_dir = get_cache_dir()?;
		let filename = request
			.cache_key
			.unwrap_or_else(|| request.url.split('/').next_back().unwrap_or("download").to_string());
		let cache_path = cache_dir.join(&filename);

		if cache_path.exists()
			&& let Ok(cached_data) = std::fs::read(&cache_path)
			&& verify_hash(&cached_data, request.blake3.clone(), request.sha256.clone(), &request.url).is_ok()
		{
			if request.extract.unwrap_or(false) {
				let extract_path = cache_dir.join(request.extract_dir.unwrap_or_else(|| format!("{}_extracted", filename)));
				if !extract_path.exists() {
					fs::create_dir_all(&extract_path).map_err(mlua::Error::external)?;
					extract_archive(&cache_path, &extract_path).map_err(mlua::Error::external)?;
				}
				return Ok(extract_path.to_string_lossy().to_string());
			} else {
				return Ok(cache_path.to_string_lossy().to_string());
			}
		}

		let config = ureq::Agent::config_builder().max_redirects(10).build();
		let agent: ureq::Agent = config.into();

		let mut response = agent
			.get(&request.url)
			.header("User-Agent", format!("forge/{}", env!("CARGO_PKG_VERSION")))
			.header("Accept", "application/octet-stream")
			.call()
			.map_err(|e| mlua::Error::RuntimeError(format!("Failed to download {}: {}", request.url, e)))?;

		if response.status() != 200 {
			return Err(mlua::Error::RuntimeError(format!(
				"HTTP {} for {}",
				response.status(),
				request.url
			)));
		}

		let mut data = Vec::new();
		response
			.body_mut()
			.as_reader()
			.read_to_end(&mut data)
			.map_err(|e| mlua::Error::RuntimeError(format!("Failed to read response: {}", e)))?;

		verify_hash(&data, request.blake3, request.sha256, &request.url)?;

		let mut file = File::create(&cache_path)
			.map_err(|e| mlua::Error::RuntimeError(format!("Failed to create cache file: {}", e)))?;
		file.write_all(&data)
			.map_err(|e| mlua::Error::RuntimeError(format!("Failed to write cache file: {}", e)))?;

		if request.extract.unwrap_or(false) {
			let extract_path = cache_dir.join(request.extract_dir.unwrap_or_else(|| format!("{}_extracted", filename)));
			fs::create_dir_all(&extract_path).map_err(mlua::Error::external)?;
			extract_archive(&cache_path, &extract_path).map_err(mlua::Error::external)?;
			Ok(extract_path.to_string_lossy().to_string())
		} else {
			Ok(cache_path.to_string_lossy().to_string())
		}
	}
}

fn get_cache_dir() -> Result<PathBuf> {
	let home = dirs::home_dir().ok_or_else(|| mlua::Error::RuntimeError("Could not find home directory".into()))?;
	let cache_dir = home.join(".forge").join("downloads");
	fs::create_dir_all(&cache_dir).map_err(mlua::Error::external)?;
	Ok(cache_dir)
}

fn verify_hash(data: &[u8], blake3: Option<String>, sha256: Option<String>, url: &str) -> Result<()> {
	if let Some(expected_blake3) = blake3 {
		let mut hasher = Blake3Hasher::new();
		hasher.update(data);
		let actual = hasher.finalize().to_hex().to_string();
		if actual != expected_blake3 {
			return Err(mlua::Error::external(ForgeError::ChecksumMismatch {
				url: url.to_string(),
				expected: expected_blake3,
				actual,
			}));
		}
	} else if let Some(expected_sha256) = sha256 {
		let mut hasher = Sha256::new();
		hasher.update(data);
		let actual = format!("{:x}", hasher.finalize());
		if actual != expected_sha256 {
			return Err(mlua::Error::external(ForgeError::ChecksumMismatch {
				url: url.to_string(),
				expected: expected_sha256,
				actual,
			}));
		}
	}
	Ok(())
}

pub fn create_http_table(lua: &Lua) -> Result<Table> {
	HttpApi::create_http_table(lua)
}
