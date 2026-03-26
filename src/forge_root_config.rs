use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ForgeRootConfigError {
	#[error("Failed to read FORGE_ROOT file: {0}")]
	Io(#[from] std::io::Error),

	#[error("Failed to parse FORGE_ROOT TOML: {0}")]
	Toml(#[from] toml::de::Error),

	#[error("Invalid configuration: {0}")]
	Invalid(String),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ForgeRootConfig {
	pub project: ProjectConfig,
	#[serde(default)]
	pub discovery: DiscoveryConfig,
	#[serde(default)]
	pub build: BuildConfig,
	#[serde(default)]
	pub toolchain: std::collections::HashMap<String, ToolchainConfig>,
	#[serde(default)]
	pub platforms: std::collections::HashMap<String, PlatformConfig>,
	#[serde(default)]
	pub profile: std::collections::HashMap<String, ProfileConfig>,
	#[serde(default)]
	pub deps: std::collections::HashMap<String, DepConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PlatformConfig {
	pub os: String,
	pub arch: String,
	pub abi: String,
	pub cpu: Option<String>,
	#[serde(default)]
	pub constraint_values: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ProfileConfig {
	pub inherits: Option<String>,
	pub opt_level: Option<u8>,
	pub debug: Option<bool>,
	pub lto: Option<bool>,
	pub strip: Option<bool>,
	pub coverage: Option<bool>,
	#[serde(default)]
	pub defines: Vec<String>,
	#[serde(default)]
	pub sanitizers: Vec<String>,
	#[serde(default)]
	pub compiler_flags: Vec<String>,
	#[serde(default)]
	pub linker_flags: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum DepConfig {
	Version(String),
	Detailed {
		version: Option<String>,
		git: Option<String>,
		rev: Option<String>,
		path: Option<String>,
	},
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ToolchainConfig {
	#[serde(default = "default_from")]
	pub from: String,
	pub version: Option<String>,
	pub url: Option<String>,
	pub sha256: Option<String>,
	pub path: Option<String>,
	pub git: Option<GitSource>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct GitSource {
	pub repo: String,
	pub rev: String,
	pub subdir: Option<String>,
}

fn default_from() -> String {
	"version".to_string()
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ProjectConfig {
	pub name: String,
	#[serde(default = "default_version")]
	pub version: String,
	pub description: Option<String>,
	pub hermetic: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DiscoveryConfig {
	#[serde(default = "default_include_patterns")]
	pub include: Vec<String>,
	#[serde(default)]
	pub exclude: Vec<String>,
	#[serde(default = "default_true")]
	pub use_gitignore: bool,
	#[serde(default = "default_max_depth")]
	pub max_depth: Option<usize>,
	#[serde(default = "default_prelude_paths")]
	pub prelude_paths: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BuildConfig {
	#[serde(default = "default_cache_dir")]
	pub cache_dir: String,
	#[serde(default)]
	pub global_env: std::collections::HashMap<String, String>,
}

impl Default for DiscoveryConfig {
	fn default() -> Self {
		Self {
			include: default_include_patterns(),
			exclude: Vec::new(),
			use_gitignore: true,
			max_depth: Some(10),
			prelude_paths: default_prelude_paths(),
		}
	}
}

impl Default for BuildConfig {
	fn default() -> Self {
		Self {
			cache_dir: default_cache_dir(),
			global_env: std::collections::HashMap::new(),
		}
	}
}

fn default_version() -> String {
	"0.1.0".to_string()
}

fn default_include_patterns() -> Vec<String> {
	vec!["src".to_string(), "lib".to_string(), "examples".to_string(), ".".to_string()]
}

fn default_true() -> bool {
	true
}

fn default_cache_dir() -> String {
	"forge-out".to_string()
}

fn default_max_depth() -> Option<usize> {
	Some(10)
}

fn default_prelude_paths() -> Vec<String> {
	vec![".forge/prelude".to_string()]
}

impl ForgeRootConfig {
	pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ForgeRootConfigError> {
		let content = std::fs::read_to_string(path)?;
		let config: ForgeRootConfig = toml::from_str(&content)?;
		config.validate()?;
		Ok(config)
	}

	pub fn validate(&self) -> Result<(), ForgeRootConfigError> {
		if self.project.name.trim().is_empty() {
			return Err(ForgeRootConfigError::Invalid("Project name cannot be empty".to_string()));
		}

		if semver::Version::parse(&self.project.version).is_err() {
			return Err(ForgeRootConfigError::Invalid(format!(
				"Invalid version format: '{}'. Must be valid semver (e.g., '1.0.0')",
				self.project.version
			)));
		}

		if self.discovery.include.is_empty() {
			return Err(ForgeRootConfigError::Invalid(
				"Discovery include patterns cannot be empty".to_string(),
			));
		}

		Ok(())
	}

	pub fn create_default(project_name: &str) -> Self {
		Self {
			project: ProjectConfig {
				name: project_name.to_string(),
				version: default_version(),
				description: None,
				hermetic: None,
			},
			discovery: DiscoveryConfig::default(),
			build: BuildConfig::default(),
			toolchain: std::collections::HashMap::new(),
			platforms: std::collections::HashMap::new(),
			profile: std::collections::HashMap::new(),
			deps: std::collections::HashMap::new(),
		}
	}

	pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ForgeRootConfigError> {
		let content = toml::to_string_pretty(self)
			.map_err(|e| ForgeRootConfigError::Invalid(format!("Failed to serialize TOML: {}", e)))?;
		std::fs::write(path, content)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_default_config() {
		let config = ForgeRootConfig::create_default("test-project");
		assert_eq!(config.project.name, "test-project");
		assert_eq!(config.project.version, "0.1.0");
		assert!(config.discovery.use_gitignore);
		assert!(config.discovery.include.contains(&"src".to_string()));
	}

	#[test]
	fn test_config_validation() {
		let mut config = ForgeRootConfig::create_default("test");

		assert!(config.validate().is_ok());

		config.project.name = "".to_string();
		assert!(config.validate().is_err());

		config.project.name = "test".to_string();
		config.project.version = "invalid-version".to_string();
		assert!(config.validate().is_err());
	}

	#[test]
	fn test_toml_serialization() {
		let config = ForgeRootConfig::create_default("test-project");
		let toml_str = toml::to_string(&config).unwrap();
		let parsed: ForgeRootConfig = toml::from_str(&toml_str).unwrap();

		assert_eq!(config.project.name, parsed.project.name);
		assert_eq!(config.discovery.use_gitignore, parsed.discovery.use_gitignore);
	}
}
