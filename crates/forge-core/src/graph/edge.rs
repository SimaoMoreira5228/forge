use crate::platform::ConfigTransition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyEdge {
	Hard,

	OrderOnly,

	Transition(ConfigTransition),

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
