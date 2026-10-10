use std::path::PathBuf;

use serde::Serialize;

use crate::label::Label;
use crate::platform::{ConfigTransition, Platform};

pub type ComponentId = petgraph::graph::NodeIndex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Component {
	pub label: Label,
	pub kind: ComponentKind,
	pub visibility: Visibility,
	pub compatible_with: Vec<String>,
	pub sources: Vec<PathBuf>,
	pub headers: Vec<PathBuf>,
	#[serde(default)]
	pub configuration: ConfigTransition,
}

impl ComponentKind {
	pub fn name(&self) -> &'static str {
		match self {
			ComponentKind::Library { .. } => "library",
			ComponentKind::Binary => "binary",
			ComponentKind::Test => "test",
			ComponentKind::Generic { .. } => "rule",
		}
	}
}

impl Component {
	pub fn is_compatible_with(&self, platform: &Platform) -> bool {
		self.compatible_with.iter().all(|p| platform.matches(p))
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ComponentKind {
	Library { link: LinkType },
	Binary,
	Test,
	Generic { command: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LinkType {
	Static,
	Shared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Visibility {
	Public,
	Package,
	Patterns(Vec<String>),
}

impl Visibility {
	pub fn allows(&self, consumer_package: &str) -> bool {
		match self {
			Visibility::Public => true,
			Visibility::Package => false,
			Visibility::Patterns(patterns) => patterns.iter().any(|p| package_pattern_matches(p, consumer_package)),
		}
	}
}

pub fn package_pattern_matches(pattern: &str, package: &str) -> bool {
	let pattern = pattern.trim_start_matches("//");
	if pattern == "..." {
		return true;
	}
	if let Some(prefix) = pattern.strip_suffix("/...") {
		return package == prefix || package.starts_with(&format!("{prefix}/"));
	}
	package == pattern
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pattern_matching() {
		assert!(package_pattern_matches("//lib/...", "lib/math"));
		assert!(package_pattern_matches("//lib/...", "lib"));
		assert!(!package_pattern_matches("//lib/...", "library"));
		assert!(package_pattern_matches("//bin", "bin"));
		assert!(!package_pattern_matches("//bin", "binary"));
		assert!(package_pattern_matches("//...", "anything/here"));
	}

	#[test]
	fn compatibility_requires_all_predicates() {
		let c = Component {
			label: Label::new("hal", "arm"),
			kind: ComponentKind::Library { link: LinkType::Static },
			visibility: Visibility::Public,
			compatible_with: vec!["os=none".into(), "arch=armv7".into()],
			sources: vec![],
			headers: vec![],
			configuration: Default::default(),
		};
		let arm = Platform {
			os: "none".into(),
			arch: "armv7".into(),
			abi: Some("eabihf".into()),
			cpu: None,
		};
		let linux = Platform::host();
		assert!(c.is_compatible_with(&arm));
		assert!(!c.is_compatible_with(&linux));
	}
}
