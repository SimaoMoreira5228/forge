use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyEdge {
	Hard,
	OrderOnly,
	ModuleImport,
}

impl Default for DependencyEdge {
	fn default() -> Self {
		DependencyEdge::Hard
	}
}

impl DependencyEdge {
	pub fn is_hard(&self) -> bool {
		matches!(self, DependencyEdge::Hard)
	}

	pub fn is_order_only(&self) -> bool {
		matches!(self, DependencyEdge::OrderOnly)
	}

	pub fn is_module_import(&self) -> bool {
		matches!(self, DependencyEdge::ModuleImport)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ComponentRef {
	Local {
		name: String,
	},
	WithTarget {
		name: String,
		target: String,
	},
}

impl ComponentRef {
	pub fn new(name: impl Into<String>) -> Self {
		ComponentRef::Local { name: name.into() }
	}

	pub fn with_target(name: impl Into<String>, target: impl Into<String>) -> Self {
		ComponentRef::WithTarget {
			name: name.into(),
			target: target.into(),
		}
	}

	pub fn name(&self) -> &str {
		match self {
			ComponentRef::Local { name } => name,
			ComponentRef::WithTarget { name, .. } => name,
		}
	}

	pub fn target(&self) -> Option<&str> {
		match self {
			ComponentRef::Local { .. } => None,
			ComponentRef::WithTarget { target, .. } => Some(target),
		}
	}
}

impl From<String> for ComponentRef {
	fn from(name: String) -> Self {
		ComponentRef::Local { name }
	}
}

impl From<&str> for ComponentRef {
	fn from(name: &str) -> Self {
		ComponentRef::Local { name: name.to_string() }
	}
}
