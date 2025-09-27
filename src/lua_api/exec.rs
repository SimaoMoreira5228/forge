use forge_macros::lua_api;
use mlua::{Lua, Table, UserData, UserDataMethods};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExecError {
	#[error("Command not found: {command}")]
	CommandNotFound {
		command: String,
	},

	#[error("Command failed: {command} - {reason}")]
	CommandFailed {
		command: String,
		reason: String,
	},

	#[error("Invalid working directory: {dir}")]
	InvalidWorkingDir {
		dir: String,
	},

	#[error("Command timeout: {command}")]
	_Timeout {
		command: String,
	},
}

#[derive(Clone)]
pub struct ExecApi;

impl UserData for ExecApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "exec")]
impl ExecApi {
	pub fn new() -> Self {
		Self
	}

	/// Execute command with optional arguments (simple version)
	fn exec(lua: &Lua, command: String, args: Option<Vec<String>>) -> mlua::Result<Table> {
		let args = args.unwrap_or_default();
		let mut cmd = Command::new(&command);
		cmd.args(&args);

		let output = cmd.output().map_err(|_| {
			mlua::Error::external(ExecError::CommandNotFound {
				command: command.clone(),
			})
		})?;

		let result = lua.create_table()?;
		result.set("success", output.status.success())?;
		result.set("exit_code", output.status.code())?;
		result.set("stdout", String::from_utf8_lossy(&output.stdout).to_string())?;
		result.set("stderr", String::from_utf8_lossy(&output.stderr).to_string())?;

		Ok(result)
	}

	/// Execute command with full configuration table
	fn run(lua: &Lua, options: Table) -> mlua::Result<Table> {
		let command: String = options.get("command")?;
		let args: Vec<String> = options.get("args").unwrap_or_default();
		let env: Option<Table> = options.get("env").ok();
		let working_dir: Option<String> = options.get("working_dir").ok();
		let _timeout: Option<f64> = options.get("timeout").ok();

		let mut cmd = Command::new(&command);
		cmd.args(&args);

		if let Some(env_table) = env {
			for pair in env_table.pairs::<String, String>() {
				let (key, value) = pair?;
				cmd.env(key, value);
			}
		}

		if let Some(dir) = working_dir {
			let dir_path = std::path::Path::new(&dir);
			if !dir_path.exists() {
				return Err(mlua::Error::external(ExecError::InvalidWorkingDir { dir: dir.clone() }));
			}
			cmd.current_dir(dir);
		}

		// TODO: Implement timeout handling in future
		// For now, execute without timeout
		let output = cmd.output().map_err(|e| {
			mlua::Error::external(ExecError::CommandFailed {
				command: command.clone(),
				reason: e.to_string(),
			})
		})?;

		let result = lua.create_table()?;
		result.set("success", output.status.success())?;
		result.set("exit_code", output.status.code())?;
		result.set("stdout", String::from_utf8_lossy(&output.stdout).to_string())?;
		result.set("stderr", String::from_utf8_lossy(&output.stderr).to_string())?;

		if !output.status.success() {
			result.set(
				"error",
				format!("Command '{}' failed with exit code {:?}", command, output.status.code()),
			)?;
		}

		Ok(result)
	}
}

pub fn create_exec_table(lua: &Lua) -> mlua::Result<Table> {
	ExecApi::create_exec_table(lua)
}
