use serde::{Deserialize, Serialize};

/// Describes how a dependency should be built relative to its consumer.
///
/// Used with `DependencyEdge::Transition` to model cross-configuration builds
/// such as host tools (code generators, proc-macro crates, build scripts) that
/// must run on the build machine regardless of the configured target platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfigTransition {
	/// Build for the machine running Forge (the build host).
	Host,
	/// Build for the execution platform (same as `Host` for local-only builds).
	Exec,
	/// Build for the configured target platform (default — no transition).
	Target,
}

impl ConfigTransition {
	pub fn is_host(&self) -> bool {
		matches!(self, ConfigTransition::Host)
	}

	pub fn is_exec(&self) -> bool {
		matches!(self, ConfigTransition::Exec)
	}

	pub fn is_target(&self) -> bool {
		matches!(self, ConfigTransition::Target)
	}
}

impl std::fmt::Display for ConfigTransition {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ConfigTransition::Host => write!(f, "host"),
			ConfigTransition::Exec => write!(f, "exec"),
			ConfigTransition::Target => write!(f, "target"),
		}
	}
}

/// The semantic meaning of a directed edge in the build dependency graph.
///
/// All variants are `Copy` — `ConfigTransition` is a fieldless enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyEdge {
	/// Standard build dependency: the target must be fully built before this
	/// component can start compiling.
	Hard,

	/// Order-only dependency: the target must *exist* before this component
	/// starts, but changes to the target do not invalidate this component's
	/// cache entry.
	OrderOnly,

	/// C++20 module import dependency.  The BMI (Binary Module Interface)
	/// for the imported module must be produced before any translation unit
	/// in this component begins compilation.
	ModuleImport,

	/// Rust procedural macro dependency.  The macro crate is compiled for the
	/// *host* platform (not the target) and then loaded into the compiler
	/// process.  The scheduler applies an implicit `host` configuration
	/// transition to the dependency.
	ProcMacro,

	/// Rust build script (`build.rs`) dependency.  The script is compiled for
	/// the host and executed before the dependent crate is compiled.  Its
	/// stdout is parsed for `cargo:` directives that influence compilation.
	BuildScript,

	/// The dependency is built with a different configuration than its
	/// consumer.  The `ConfigTransition` value describes the target
	/// configuration for the dependency.
	Transition(ConfigTransition),
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

	pub fn is_proc_macro(&self) -> bool {
		matches!(self, DependencyEdge::ProcMacro)
	}

	pub fn is_build_script(&self) -> bool {
		matches!(self, DependencyEdge::BuildScript)
	}

	pub fn is_transition(&self) -> bool {
		matches!(self, DependencyEdge::Transition(_))
	}

	pub fn transition(&self) -> Option<ConfigTransition> {
		match self {
			DependencyEdge::Transition(t) => Some(*t),
			_ => None,
		}
	}

	/// Human-readable label for DOT graph output.
	pub fn dot_label(&self) -> &'static str {
		match self {
			DependencyEdge::Hard => "",
			DependencyEdge::OrderOnly => "order-only",
			DependencyEdge::ModuleImport => "module",
			DependencyEdge::ProcMacro => "proc-macro",
			DependencyEdge::BuildScript => "build-script",
			DependencyEdge::Transition(ConfigTransition::Host) => "transition:host",
			DependencyEdge::Transition(ConfigTransition::Exec) => "transition:exec",
			DependencyEdge::Transition(ConfigTransition::Target) => "transition:target",
		}
	}
}

impl std::fmt::Display for DependencyEdge {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.dot_label())
	}
}

/// A reference to another component, either by name alone or name + target.
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
