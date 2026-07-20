use std::collections::BTreeMap;
use std::fmt;

use crate::label::Label;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForgeLock {
	pub version: u32,
	pub packages: Vec<LockedPackage>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LockedPackage {
	pub name: String,
	pub version: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub source: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub checksum: Option<String>,
	#[serde(default)]
	pub dependencies: Vec<String>,
}

impl ForgeLock {
	pub fn new() -> Self {
		Self {
			version: 1,
			packages: Vec::new(),
		}
	}

	pub fn from_solution(solution: &BTreeMap<Label, String>) -> Self {
		let packages: Vec<LockedPackage> = solution
			.iter()
			.map(|(name, version)| LockedPackage {
				name: name.name().to_string(),
				version: version.clone(),
				source: None,
				checksum: None,
				dependencies: Vec::new(),
			})
			.collect();
		Self { version: 1, packages }
	}

	pub fn parse(text: &str) -> Result<Self, String> {
		toml::from_str(text).map_err(|e| format!("invalid forge.lock: {e}"))
	}

	pub fn to_toml(&self) -> Result<String, String> {
		toml::to_string_pretty(self).map_err(|e| format!("failed to serialize forge.lock: {e}"))
	}

	pub fn get(&self, name: &str) -> Option<&LockedPackage> {
		self.packages.iter().find(|p| p.name == name)
	}

	pub fn iter(&self) -> impl Iterator<Item = &LockedPackage> {
		self.packages.iter()
	}
}

impl Default for ForgeLock {
	fn default() -> Self {
		Self::new()
	}
}

impl fmt::Display for ForgeLock {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for pkg in &self.packages {
			writeln!(f, "{}@{}", pkg.name, pkg.version)?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_and_serialize() {
		let text = r#"
version = 1

[[packages]]
name = "serde"
version = "1.0.200"

[[packages]]
name = "toml"
version = "0.8.0"
"#;
		let lock = ForgeLock::parse(text).unwrap();
		assert_eq!(lock.packages.len(), 2);
		assert_eq!(lock.packages[0].name, "serde");
		assert_eq!(lock.packages[1].version, "0.8.0");

		let serialized = lock.to_toml().unwrap();
		assert!(serialized.contains("serde"));
		assert!(serialized.contains("1.0.200"));
	}

	#[test]
	fn from_solution() {
		let mut solution = BTreeMap::new();
		solution.insert(Label::parse("serde", "").unwrap(), "1.0.200".into());
		solution.insert(Label::parse("toml", "").unwrap(), "0.8.0".into());
		let lock = ForgeLock::from_solution(&solution);
		assert_eq!(lock.packages.len(), 2);
		assert_eq!(lock.packages[0].name, "serde");
		assert_eq!(lock.packages[0].version, "1.0.200");
	}
}
