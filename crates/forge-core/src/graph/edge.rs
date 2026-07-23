use crate::platform::ConfigTransition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyEdge {
	/// Must complete before this starts.
	Hard,
	/// Must exist before this starts; no invalidation on rebuild.
	OrderOnly,
	/// Dependency built under a different configuration.
	Transition(ConfigTransition),
	/// Cell-defined edge kind (e.g. `"proc_macro"`, `"module_import"`).
	/// The core only knows it blocks execution; the meaning belongs to the cell.
	Tagged(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyDecl {
	pub label: String,
	pub edge: DependencyEdge,
}

impl DependencyEdge {
	pub fn blocks_execution(&self) -> bool {
		!matches!(self, DependencyEdge::OrderOnly)
	}
}
