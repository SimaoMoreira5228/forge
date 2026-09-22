use std::collections::BTreeMap;

use pubgrub::{OfflineDependencyProvider, Ranges, resolve};

use super::version::Version;

pub type VersionRange = Ranges<Version>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyRequirement {
	pub name: String,
	pub range: VersionRange,
}

impl DependencyRequirement {
	pub fn new(name: impl Into<String>, range: VersionRange) -> Self {
		Self {
			name: name.into(),
			range,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageCandidate {
	pub name: String,
	pub version: Version,
	pub source: Option<String>,
	pub checksum: Option<String>,
	pub dependencies: Vec<DependencyRequirement>,
}

impl PackageCandidate {
	pub fn new(name: impl Into<String>, version: Version) -> Self {
		Self {
			name: name.into(),
			version,
			source: None,
			checksum: None,
			dependencies: Vec::new(),
		}
	}

	pub fn from_source(mut self, source: impl Into<String>, checksum: impl Into<String>) -> Self {
		self.source = Some(source.into());
		self.checksum = Some(checksum.into());
		self
	}

	pub fn with_dependencies(mut self, dependencies: impl IntoIterator<Item = DependencyRequirement>) -> Self {
		self.dependencies = dependencies.into_iter().collect();
		self
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
	pub name: String,
	pub version: Version,
	pub source: Option<String>,
	pub checksum: Option<String>,
	pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedGraph {
	pub packages: BTreeMap<String, ResolvedPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError(pub String);

impl std::fmt::Display for ResolveError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl std::error::Error for ResolveError {}

pub fn solve(
	root: impl Into<String>,
	requirements: impl IntoIterator<Item = DependencyRequirement>,
	candidates: impl IntoIterator<Item = PackageCandidate>,
) -> Result<ResolvedGraph, ResolveError> {
	let root = root.into();
	let mut provider = OfflineDependencyProvider::<String, VersionRange>::new();
	let root_version = Version::new(0, 0, 0);
	let requirements: Vec<_> = requirements.into_iter().collect();
	let candidates: Vec<_> = candidates.into_iter().collect();
	provider.add_dependencies(
		root.clone(),
		root_version.clone(),
		requirements
			.iter()
			.map(|dependency| (dependency.name.clone(), dependency.range.clone())),
	);

	let mut seen = BTreeMap::new();
	for candidate in &candidates {
		if candidate.name == root {
			return Err(ResolveError(format!("package name `{root}` is reserved")));
		}
		if seen.insert((candidate.name.clone(), candidate.version.clone()), ()).is_some() {
			return Err(ResolveError(format!(
				"duplicate candidate `{}@{}`",
				candidate.name, candidate.version
			)));
		}
		provider.add_dependencies(
			candidate.name.clone(),
			candidate.version.clone(),
			candidate
				.dependencies
				.iter()
				.map(|dependency| (dependency.name.clone(), dependency.range.clone())),
		);
	}

	let selected = resolve(&provider, root.clone(), root_version)
		.map_err(|error| ResolveError(format!("dependency resolution failed: {error:?}")))?;
	let selected: BTreeMap<_, _> = selected.into_iter().collect();
	let packages = selected
		.iter()
		.filter(|(name, _)| *name != &root)
		.map(|(name, version)| {
			let candidate = candidates
				.iter()
				.find(|candidate| candidate.name == *name && candidate.version == *version)
				.expect("selected package must have a registered candidate");
			(
				name.clone(),
				ResolvedPackage {
					name: name.clone(),
					version: version.clone(),
					source: candidate.source.clone(),
					checksum: candidate.checksum.clone(),
					dependencies: candidate
						.dependencies
						.iter()
						.map(|dependency| {
							selected
								.get(&dependency.name)
								.map(|version| format!("{} {}", dependency.name, version))
								.unwrap_or_else(|| dependency.name.clone())
						})
						.collect(),
				},
			)
		})
		.collect();
	Ok(ResolvedGraph { packages })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn version(major: u64) -> Version {
		Version::new(major, 0, 0)
	}

	#[test]
	fn resolves_transitive_diamond_to_one_version() {
		let solution = solve(
			"app",
			[
				DependencyRequirement::new("left", Ranges::full()),
				DependencyRequirement::new("right", Ranges::full()),
			],
			[
				PackageCandidate::new("left", version(1))
					.with_dependencies([DependencyRequirement::new("shared", Ranges::between(version(1), version(3)))]),
				PackageCandidate::new("right", version(1))
					.with_dependencies([DependencyRequirement::new("shared", Ranges::between(version(2), version(4)))]),
				PackageCandidate::new("shared", version(1)),
				PackageCandidate::new("shared", version(2)),
			],
		)
		.unwrap();

		assert_eq!(solution.packages["shared"].version, version(2));
	}

	#[test]
	fn reports_unsatisfied_transitive_dependency() {
		let error = solve(
			"app",
			[DependencyRequirement::new("top", Ranges::full())],
			[PackageCandidate::new("top", version(1))
				.with_dependencies([DependencyRequirement::new("missing", Ranges::full())])],
		)
		.unwrap_err();

		assert!(error.0.contains("missing"));
	}

	#[test]
	fn resolution_is_deterministic_for_the_same_requirements_and_candidates() {
		let requirements = [
			DependencyRequirement::new("left", Ranges::between(version(1), version(3))),
			DependencyRequirement::new("right", Ranges::between(version(1), version(3))),
		];
		let candidates = [
			PackageCandidate::new("left", version(1))
				.from_source("https://example.invalid/left", "left-sha")
				.with_dependencies([DependencyRequirement::new("shared", Ranges::full())]),
			PackageCandidate::new("right", version(1))
				.from_source("https://example.invalid/right", "right-sha")
				.with_dependencies([DependencyRequirement::new("shared", Ranges::between(version(2), version(4)))]),
			PackageCandidate::new("shared", version(1)).from_source("https://example.invalid/shared-1", "sha-1"),
			PackageCandidate::new("shared", version(2)).from_source("https://example.invalid/shared-2", "sha-2"),
		];
		let expected =
			crate::resolver::ForgeLock::from_resolved(&solve("app", requirements.clone(), candidates.clone()).unwrap())
				.to_toml()
				.unwrap();

		for _ in 0..16 {
			let graph = solve("app", requirements.clone(), candidates.clone()).unwrap();
			assert_eq!(graph.packages.len(), 3);
			assert_eq!(graph.packages["shared"].version, version(2));
			assert_eq!(crate::resolver::ForgeLock::from_resolved(&graph).to_toml().unwrap(), expected);
		}
	}
}
