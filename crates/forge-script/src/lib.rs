pub mod cells;
pub mod dep_graph;
pub mod discover;
pub mod document;
pub mod glob;
pub mod parser;
pub mod register;
pub mod rhai_rt;
pub mod std_cells;
pub mod workspace;

pub use discover::discover_packages;
pub use document::{SourceExpr, TargetDecl, TargetKind, Value};
pub use parser::parse_forge_toml;
pub use register::{DeclMap, load_workspace, register_package};
pub use workspace::{ToolchainSelection, WorkspaceConfig};

pub mod fmt;
#[cfg(test)]
mod parser_tests;
#[cfg(test)]
mod rhai_tests;
