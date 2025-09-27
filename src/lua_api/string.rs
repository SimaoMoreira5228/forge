use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};

#[derive(Clone)]
pub struct StringApi;

impl UserData for StringApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "string")]
impl StringApi {
	pub fn new() -> Self {
		Self
	}

	/// Split string by delimiter
	fn split(input: String, delimiter: String) -> Result<Vec<String>> {
		let parts: Vec<String> = input.split(&delimiter).map(|s| s.to_string()).collect();
		Ok(parts)
	}

	/// Join strings with delimiter
	fn join(parts: Vec<String>, delimiter: String) -> Result<String> {
		Ok(parts.join(&delimiter))
	}

	/// Trim whitespace
	fn trim(input: String) -> Result<String> {
		Ok(input.trim().to_string())
	}

	/// Trim whitespace from start
	fn trim_start(input: String) -> Result<String> {
		Ok(input.trim_start().to_string())
	}

	/// Trim whitespace from end
	fn trim_end(input: String) -> Result<String> {
		Ok(input.trim_end().to_string())
	}

	/// Check if string starts with prefix
	fn starts_with(input: String, prefix: String) -> Result<bool> {
		Ok(input.starts_with(&prefix))
	}

	/// Check if string ends with suffix
	fn ends_with(input: String, suffix: String) -> Result<bool> {
		Ok(input.ends_with(&suffix))
	}

	/// Replace all occurrences of a substring
	fn replace(input: String, from: String, to: String) -> Result<String> {
		Ok(input.replace(&from, &to))
	}

	/// Convert to lowercase
	fn to_lower(input: String) -> Result<String> {
		Ok(input.to_lowercase())
	}

	/// Convert to uppercase
	fn to_upper(input: String) -> Result<String> {
		Ok(input.to_uppercase())
	}

	/// Check if string contains substring
	fn contains(input: String, needle: String) -> Result<bool> {
		Ok(input.contains(&needle))
	}

	/// Shell escape - properly escape arguments for shell commands
	fn escape_shell(input: String) -> Result<String> {
		if input.contains('\'') {
			Ok(format!("'{}'", input.replace('\'', r#"'"'"'"#)))
		} else if input.contains(' ') || input.contains('\t') || input.contains('\n') || input.is_empty() {
			Ok(format!("'{}'", input))
		} else {
			Ok(input)
		}
	}

	/// Pad string to specified length on the left
	fn pad_left(input: String, length: usize, pad_char: Option<String>) -> Result<String> {
		let pad_char = pad_char.unwrap_or_else(|| " ".to_string());
		let pad_char = pad_char.chars().next().unwrap_or(' ');

		if input.len() >= length {
			Ok(input)
		} else {
			let padding = pad_char.to_string().repeat(length - input.len());
			Ok(format!("{}{}", padding, input))
		}
	}

	/// Pad string to specified length on the right
	fn pad_right(input: String, length: usize, pad_char: Option<String>) -> Result<String> {
		let pad_char = pad_char.unwrap_or_else(|| " ".to_string());
		let pad_char = pad_char.chars().next().unwrap_or(' ');

		if input.len() >= length {
			Ok(input)
		} else {
			let padding = pad_char.to_string().repeat(length - input.len());
			Ok(format!("{}{}", input, padding))
		}
	}
}

pub fn create_string_table(lua: &Lua) -> Result<Table> {
	StringApi::create_string_table(lua)
}
