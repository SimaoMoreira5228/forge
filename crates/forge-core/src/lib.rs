pub mod action;
pub mod depfile;
pub mod graph;
pub mod label;
pub mod platform;
pub mod profile;
pub mod resolver;
pub mod toolchain;

pub use action::spec::{ActionSpec, ArgumentFile, EnvironmentFile, OutputDeclaration, OutputKind};
pub use graph::build_graph::BuildGraph;
pub use graph::component::{Component, ComponentId, ComponentKind, LinkType, Visibility};
pub use graph::edge::{DependencyDecl, DependencyEdge};
pub use label::Label;
pub use platform::{ConfigTransition, Platform};
pub use profile::{DebugInfo, Lto, OptLevel, Profile, Strip};
pub use resolver::{
	DependencyRequest, DependencyRequirement, LockedSource, PackageCandidate, ResolveError, ResolvedGraph, ResolvedPackage,
	Version, VersionRange, solve,
};
pub use toolchain::Catalog;
pub use toolchain::catalog::TargetUrl;
