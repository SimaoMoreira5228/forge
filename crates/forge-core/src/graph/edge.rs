use crate::platform::ConfigTransition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyEdge {
	/// Must complete before this starts.
	Hard,
	/// Must exist before this starts; no invalidation on rebuild.
	OrderOnly,
	/// C++20 module BMI barrier.
	ModuleImport,
	/// Host-compiled proc macro loaded by rustc.
	ProcMacro,
	/// Build script executed before its dependent.
	BuildScript,
	/// Dependency built under a different configuration.
	Transition(ConfigTransition),
}

impl DependencyEdge {
	pub fn blocks_execution(&self) -> bool {
		!matches!(self, DependencyEdge::OrderOnly)
	}
}
