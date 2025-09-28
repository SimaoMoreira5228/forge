use thiserror::Error;

#[derive(Error, Debug)]
pub enum ForgeError {
	#[error(
		"FORGE_ROOT configuration file not found in '{path}'\n\n\
		Suggestion: Create a FORGE_ROOT file with:\n\
		[project]\n\
		name = \"my-project\"\n\
		version = \"1.0.0\"\n\n\
		[discovery]\n\
		include = [\"src\", \"lib\"]\n"
	)]
	ForgeRootNotFound {
		path: String,
	},

	#[error("FORGE_ROOT configuration error: {0}")]
	ForgeRootConfigError(#[from] crate::forge_root_config::ForgeRootConfigError),

	#[error(
		"No FORGE files found in project\n\n\
		Searched in: {searched_paths}\n\n\
		Suggestion: Create FORGE files in your source directories or update the 'include' patterns in FORGE_ROOT"
	)]
	NoForgeFilesFound {
		searched_paths: String,
	},

	#[error(
		"Lua execution error in {file}: {error}\n\nSuggestion: Check your FORGE file syntax and ensure all required variables are defined."
	)]
	LuaError {
		file: String,
		error: mlua::Error,
	},

	#[error("Lua execution error: {0}")]
	LuaExecutionError(#[from] mlua::Error),

	#[error("I/O error: {0}")]
	IoError(#[from] std::io::Error),

	#[error("System time error: {0}")]
	SystemTimeError(#[from] std::time::SystemTimeError),

	#[error("HTTP request failed: {0}")]
	RequestError(#[from] ureq::Error),

	#[error("Archive extraction failed: {0}")]
	ExtractionError(String),

	#[error(
		"Checksum mismatch for {url}. Expected {expected}, got {actual}\n\nSuggestion: The downloaded file may be corrupted. Try cleaning the cache and rebuilding."
	)]
	ChecksumMismatch {
		url: String,
		expected: String,
		actual: String,
	},

	#[error(
		"Build failed for rule '{rule}': {error}\n\nSuggestion: Check the command, arguments, and input files for rule '{rule}'."
	)]
	BuildFailed {
		rule: String,
		error: String,
	},

	#[error(
		"Prelude directory not found at '{0}'\n\nSuggestion: Ensure the prelude directory exists and contains the required build system modules."
	)]
	PreludeNotFound(String),

	#[error(
		"Circular dependency detected: {cycle}\n\nSuggestion: Remove one of the dependencies in the cycle: {suggestions}"
	)]
	CircularDependency {
		cycle: String,
		suggestions: String,
	},

	#[error("Dependency version conflict: {conflict}\n\nSuggestion: {suggestion}")]
	DependencyConflict {
		conflict: String,
		suggestion: String,
	},

	#[error("Invalid FORGE file: {file}\n\nError: {error}\n\nSuggestion: {suggestion}")]
	InvalidForgeFile {
		file: String,
		error: String,
		suggestion: String,
	},

	#[error(transparent)]
	Other(#[from] anyhow::Error),
}

impl From<ForgeError> for mlua::Error {
	fn from(err: ForgeError) -> Self {
		mlua::Error::external(err)
	}
}
