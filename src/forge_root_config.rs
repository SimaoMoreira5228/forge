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
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ProjectConfig {
	pub name: String,
	#[serde(default = "default_version")]
	pub version: String,
	pub description: Option<String>,
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
			},
			discovery: DiscoveryConfig::default(),
			build: BuildConfig::default(),
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
