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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedSource {
	pub name: String,
	pub version: String,
	pub url: String,
	pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRequest {
	pub name: String,
	pub version: String,
	pub source: String,
	pub checksum: String,
	pub dependencies: Vec<String>,
}

impl DependencyRequest {
	pub fn new(
		name: impl Into<String>,
		version: impl Into<String>,
		source: impl Into<String>,
		checksum: impl Into<String>,
	) -> Self {
		Self {
			name: name.into(),
			version: version.into(),
			source: source.into(),
			checksum: checksum.into(),
			dependencies: Vec::new(),
		}
	}

	pub fn with_dependencies(mut self, dependencies: impl IntoIterator<Item = impl Into<String>>) -> Self {
		self.dependencies = dependencies.into_iter().map(Into::into).collect();
		self
	}
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

	pub fn from_resolved(resolved: &crate::resolver::ResolvedGraph) -> Self {
		Self {
			version: 1,
			packages: resolved
				.packages
				.values()
				.map(|package| LockedPackage {
					name: package.name.clone(),
					version: package.version.to_string(),
					source: package.source.clone(),
					checksum: package.checksum.clone(),
					dependencies: package.dependencies.clone(),
				})
				.collect(),
		}
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

	pub fn sources(&self) -> Result<Vec<LockedSource>, String> {
		let order = self.dependency_order()?;
		order
			.into_iter()
			.map(|package| {
				Ok(LockedSource {
					name: package.name.clone(),
					version: package.version.clone(),
					url: package
						.source
						.clone()
						.ok_or_else(|| format!("{}@{} has no source URL", package.name, package.version))?,
					checksum: package
						.checksum
						.clone()
						.ok_or_else(|| format!("{}@{} has no sha256 checksum", package.name, package.version))?,
				})
			})
			.collect()
	}

	pub fn dependency_order(&self) -> Result<Vec<&LockedPackage>, String> {
		let mut by_versioned = BTreeMap::new();
		let mut by_name: BTreeMap<&str, Vec<&LockedPackage>> = BTreeMap::new();
		for package in &self.packages {
			let key = format!("{}@{}", package.name, package.version);
			if by_versioned.insert(key, package).is_some() {
				return Err(format!("duplicate locked package `{}@{}`", package.name, package.version));
			}
			by_name.entry(package.name.as_str()).or_default().push(package);
		}
		let mut state = BTreeMap::new();
		let mut order = Vec::new();
		for package in &self.packages {
			visit(package, &by_versioned, &by_name, &mut state, &mut order)?;
		}
		Ok(order)
	}

	pub fn from_requests(requests: impl IntoIterator<Item = DependencyRequest>) -> Self {
		Self {
			version: 1,
			packages: requests
				.into_iter()
				.map(|request| LockedPackage {
					name: request.name,
					version: request.version,
					source: Some(request.source),
					checksum: Some(request.checksum),
					dependencies: request.dependencies,
				})
				.collect(),
		}
	}
}

fn visit<'a>(
	package: &'a LockedPackage,
	by_versioned: &BTreeMap<String, &'a LockedPackage>,
	by_name: &BTreeMap<&str, Vec<&'a LockedPackage>>,
	state: &mut BTreeMap<String, bool>,
	order: &mut Vec<&'a LockedPackage>,
) -> Result<(), String> {
	let key = format!("{}@{}", package.name, package.version);
	match state.get(&key) {
		Some(true) => return Err(format!("dependency cycle includes `{}`", key)),
		Some(false) => return Ok(()),
		None => {}
	}
	state.insert(key.clone(), true);
	for dep_str in &package.dependencies {
		let (dep_name, dep_version) = parse_dependency(dep_str);
		let dep_pkg = if let Some(version) = dep_version {
			by_versioned.get(&format!("{}@{}", dep_name, version)).copied().or_else(|| {
				by_name
					.get(dep_name)
					.and_then(|v| v.iter().find(|p| p.version == version).copied())
			})
		} else {
			by_name.get(dep_name).and_then(|v| v.first().copied())
		}
		.ok_or_else(|| format!("{}@{} depends on missing `{dep_str}`", package.name, package.version))?;
		visit(dep_pkg, by_versioned, by_name, state, order)?;
	}
	state.insert(key, false);
	order.push(package);
	Ok(())
}

fn parse_dependency(input: &str) -> (&str, Option<&str>) {
	let mut parts = input.splitn(2, ' ');
	let name = parts.next().unwrap_or(input);
	let version = parts.next();
	(name, version)
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

	#[test]
	fn sources_require_verified_locations() {
		let lock = ForgeLock {
			version: 1,
			packages: vec![LockedPackage {
				name: "demo".into(),
				version: "1.0.0".into(),
				source: Some("https://example.invalid/demo".into()),
				checksum: Some("abc".into()),
				dependencies: Vec::new(),
			}],
		};
		assert_eq!(lock.sources().unwrap()[0].url, "https://example.invalid/demo");
	}

	#[test]
	fn requests_create_generic_lock_entries() {
		let lock = ForgeLock::from_requests([DependencyRequest::new(
			"demo",
			"1.0.0",
			"https://example.invalid/demo.tar.gz",
			"abc",
		)]);
		assert_eq!(lock.sources().unwrap()[0].name, "demo");
	}

	#[test]
	fn resolved_graph_becomes_a_transitive_lockfile() {
		let resolved = crate::resolver::solve(
			"app",
			[crate::resolver::DependencyRequirement::new("top", pubgrub::Ranges::full())],
			[
				crate::resolver::PackageCandidate::new("top", crate::resolver::version::Version::new(1, 0, 0))
					.from_source("https://example.invalid/top.tar", "top-sha")
					.with_dependencies([crate::resolver::DependencyRequirement::new("leaf", pubgrub::Ranges::full())]),
				crate::resolver::PackageCandidate::new("leaf", crate::resolver::version::Version::new(1, 0, 0))
					.from_source("https://example.invalid/leaf.tar", "leaf-sha"),
			],
		)
		.unwrap();
		let lock = ForgeLock::from_resolved(&resolved);
		assert_eq!(lock.get("top").unwrap().dependencies, ["leaf 1.0.0"]);
		assert_eq!(lock.dependency_order().unwrap().len(), 2);
		assert_eq!(lock.sources().unwrap()[0].name, "leaf");
	}

	#[test]
	fn dependency_order_covers_transitive_graphs_and_rejects_cycles() {
		let make = |name: &str, dependencies: Vec<String>| LockedPackage {
			name: name.into(),
			version: "1".into(),
			source: Some(format!("https://example.invalid/{name}")),
			checksum: Some("abc".into()),
			dependencies,
		};
		let lock = ForgeLock {
			version: 1,
			packages: vec![
				make("app", vec!["mid".into()]),
				make("mid", vec!["base".into()]),
				make("base", vec![]),
			],
		};
		let order: Vec<_> = lock
			.dependency_order()
			.unwrap()
			.into_iter()
			.map(|p| p.name.as_str())
			.collect();
		assert_eq!(order, ["base", "mid", "app"]);

		let cycle = ForgeLock {
			version: 1,
			packages: vec![make("a", vec!["b".into()]), make("b", vec!["a".into()])],
		};
		assert!(cycle.dependency_order().is_err());
	}
}
