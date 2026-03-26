use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::graph::{ComponentRef, ConfigTransition, DependencyEdge};

static NEXT_COMPONENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComponentId(pub u64);

impl ComponentId {
	pub fn new() -> Self {
		Self(NEXT_COMPONENT_ID.fetch_add(1, Ordering::SeqCst))
	}
}

impl Default for ComponentId {
	fn default() -> Self {
		Self::new()
	}
}

/// Opaque identity of the `FORGE` file (package) that declares a component.
///
/// The canonical form is the absolute path of the `FORGE` file's parent
/// directory, stored as a `String` for easy serialization and pattern matching.
/// Components with the same `PackageId` live in the same package and can
/// always see each other regardless of visibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId(pub String);

impl PackageId {
	pub fn new(path: impl Into<String>) -> Self {
		Self(path.into())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Returns true if `other` is the same package or a sub-package of this one.
	pub fn contains(&self, other: &PackageId) -> bool {
		other.0.starts_with(&self.0)
	}
}

impl Default for PackageId {
	fn default() -> Self {
		Self(String::from("<unknown>"))
	}
}

impl std::fmt::Display for PackageId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// A reference to a platform constraint (e.g. `"//platforms:embedded_arm"`).
///
/// Constraints restrict the set of target platforms a component can be built
/// for.  A component with a non-empty `compatible_with` list can only be
/// built for targets that satisfy *all* listed constraints.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConstraintRef(pub String);

impl ConstraintRef {
	pub fn new(s: impl Into<String>) -> Self {
		Self(s.into())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Display for ConstraintRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Controls which other components may declare a dependency on this component.
///
/// Visibility is checked at graph construction time — before the rule engine
/// runs and before any compiler is invoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
	/// Any component in the workspace may depend on this component.
	Public,

	/// Only components in the same package (same `FORGE` file directory) may
	/// depend on this component.
	Package,

	/// Only components declared in the same `FORGE` file may depend on this
	/// component.  Because `PackageId` is keyed on the directory, `Private`
	/// is currently equivalent to `Package`.  In the future it will be
	/// tightened to per-file identity when multi-file package support lands.
	Private,

	/// Only components whose `PackageId` matches one of the listed target
	/// patterns may depend on this component.
	///
	/// Patterns follow Bazel-style notation:
	/// - `//lib/...`  — any package under `//lib/`
	/// - `//lib:foo`  — the specific target `foo` in `//lib`
	Restricted(Vec<String>),
}

impl Visibility {
	/// Returns `true` if this visibility allows access from `from_package`.
	///
	/// `own_package` is the package of the component *being accessed*.
	pub fn allows(&self, from_package: &PackageId, own_package: &PackageId) -> bool {
		match self {
			Visibility::Public => true,
			Visibility::Package | Visibility::Private => from_package == own_package,
			Visibility::Restricted(patterns) => {
				patterns.iter().any(|p| visibility_pattern_matches(p, from_package.as_str()))
			}
		}
	}

	pub fn is_public(&self) -> bool {
		matches!(self, Visibility::Public)
	}
}

impl Default for Visibility {
	fn default() -> Self {
		Visibility::Public
	}
}

impl std::fmt::Display for Visibility {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Visibility::Public => write!(f, "public"),
			Visibility::Package => write!(f, "package"),
			Visibility::Private => write!(f, "private"),
			Visibility::Restricted(patterns) => {
				write!(f, "restricted({})", patterns.join(", "))
			}
		}
	}
}

/// Match a visibility pattern against a package path.
///
/// Patterns:
/// - `//lib/...` — any path that starts with `lib/` (after stripping `//`)
/// - `//lib:something` — the package `lib` exactly
/// - `//lib` — the package `lib` exactly
fn visibility_pattern_matches(pattern: &str, package_path: &str) -> bool {
	let normalized = pattern.trim_start_matches("//");
	if let Some(prefix) = normalized.strip_suffix("/...") {
		// Recursive glob: match any path starting with the prefix
		package_path.starts_with(prefix)
	} else if let Some(pkg) = normalized.split(':').next() {
		// Exact package or label match
		package_path == pkg || package_path.starts_with(&format!("{}/", pkg))
	} else {
		package_path == normalized
	}
}

// ---------------------------------------------------------------------------
// Component types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum LinkType {
	Static,
	Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComponentType {
	Library {
		link_type: LinkType,
	},
	Binary,
	Module {
		module_name: String,
	},
	Custom {
		command: String,
		args: Vec<String>,
	},
	Test {
		test_kind: TestKind,
		executable: Option<PathBuf>,
		command: Option<Vec<String>>,
		args: Vec<String>,
		data: Vec<PathBuf>,
		env: HashMap<String, String>,
		timeout_secs: u64,
		size: TestSize,
		tags: Vec<String>,
	},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TestKind {
	#[default]
	Unit,
	Integration,
	E2E,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TestSize {
	#[default]
	Small,
	Medium,
	Large,
}

impl ComponentType {
	pub fn is_library(&self) -> bool {
		matches!(self, ComponentType::Library { .. })
	}

	pub fn is_binary(&self) -> bool {
		matches!(self, ComponentType::Binary)
	}

	pub fn is_module(&self) -> bool {
		matches!(self, ComponentType::Module { .. })
	}

	pub fn is_test(&self) -> bool {
		matches!(self, ComponentType::Test { .. })
	}

	pub fn is_custom(&self) -> bool {
		matches!(self, ComponentType::Custom { .. })
	}

	/// Short name used in DOT graph coloring and query output.
	pub fn kind_name(&self) -> &'static str {
		match self {
			ComponentType::Library { .. } => "library",
			ComponentType::Binary => "binary",
			ComponentType::Module { .. } => "module",
			ComponentType::Custom { .. } => "custom",
			ComponentType::Test { .. } => "test",
		}
	}
}

impl Default for ComponentType {
	fn default() -> Self {
		ComponentType::Library {
			link_type: LinkType::Static,
		}
	}
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// A single build target declared in a `FORGE` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
	pub id: ComponentId,

	/// Short name of this component, unique within its package + target pair.
	pub name: String,

	/// Kind and type-specific metadata.
	pub component_type: ComponentType,

	/// Which build target (platform triple alias) this component is configured for.
	pub target_name: String,

	/// The package that owns this component (path of its `FORGE` file's directory).
	pub package: PackageId,

	/// Visibility — who is allowed to declare a dependency on this component.
	pub visibility: Visibility,

	/// Platform constraints.  If non-empty, the component can only be built for
	/// targets that satisfy all listed constraints.
	pub compatible_with: Vec<ConstraintRef>,

	// Build inputs / outputs
	pub sources: Vec<PathBuf>,
	pub outputs: Vec<PathBuf>,
	pub include_dirs: Vec<PathBuf>,
	pub defines: Vec<(String, Option<String>)>,
	pub compiler_flags: Vec<String>,
	pub linker_flags: Vec<String>,
	pub system_libs: Vec<String>,
	pub workdir: PathBuf,
	pub env: HashMap<String, String>,
	/// Configuration transition for this component (e.g., host/target).
	pub exec_cfg: Option<ConfigTransition>,

	/// Unresolved dependencies to be processed after all components are loaded.
	pub dependencies: Vec<(ComponentRef, DependencyEdge)>,
}

impl Component {
	pub fn new(name: impl Into<String>, target_name: impl Into<String>) -> Self {
		let workdir = std::env::current_dir().unwrap_or_default();
		let package = PackageId::new(workdir.to_string_lossy().as_ref());
		Self {
			id: ComponentId::new(),
			name: name.into(),
			component_type: ComponentType::default(),
			target_name: target_name.into(),
			package,
			visibility: Visibility::default(),
			compatible_with: Vec::new(),
			exec_cfg: None,
			sources: Vec::new(),
			outputs: Vec::new(),
			include_dirs: Vec::new(),
			defines: Vec::new(),
			compiler_flags: Vec::new(),
			linker_flags: Vec::new(),
			system_libs: Vec::new(),
			workdir,
			env: HashMap::new(),
			dependencies: Vec::new(),
		}
	}

	pub fn with_dependencies(mut self, deps: Vec<(ComponentRef, DependencyEdge)>) -> Self {
		self.dependencies = deps;
		self
	}

	pub fn library(name: impl Into<String>, target_name: impl Into<String>) -> Self {
		let mut component = Self::new(name, target_name);
		component.component_type = ComponentType::Library {
			link_type: LinkType::Static,
		};
		component
	}

	pub fn binary(name: impl Into<String>, target_name: impl Into<String>) -> Self {
		let mut component = Self::new(name, target_name);
		component.component_type = ComponentType::Binary;
		component
	}

	pub fn custom(
		name: impl Into<String>,
		target_name: impl Into<String>,
		command: String,
		args: Vec<String>,
	) -> Self {
		let mut component = Self::new(name, target_name);
		component.component_type = ComponentType::Custom { command, args };
		component
	}

	pub fn test(
		name: impl Into<String>,
		target_name: impl Into<String>,
		executable: Option<PathBuf>,
		command: Option<Vec<String>>,
	) -> Self {
		let mut component = Self::new(name, target_name);
		component.component_type = ComponentType::Test {
			test_kind: TestKind::Unit,
			executable,
			command,
			args: Vec::new(),
			data: Vec::new(),
			env: HashMap::new(),
			timeout_secs: 300,
			size: TestSize::Small,
			tags: Vec::new(),
		};
		component
	}

	// -----------------------------------------------------------------------
	// Builder methods
	// -----------------------------------------------------------------------

	pub fn with_package(mut self, package: PackageId) -> Self {
		self.package = package;
		self
	}

	pub fn with_visibility(mut self, visibility: Visibility) -> Self {
		self.visibility = visibility;
		self
	}

	pub fn with_compatible_with(mut self, constraints: Vec<ConstraintRef>) -> Self {
		self.compatible_with = constraints;
		self
	}

	pub fn with_exec_cfg(mut self, exec_cfg: Option<ConfigTransition>) -> Self {
		self.exec_cfg = exec_cfg;
		self
	}

	pub fn with_sources(mut self, sources: Vec<PathBuf>) -> Self {
		self.sources = sources;
		self
	}

	pub fn with_outputs(mut self, outputs: Vec<PathBuf>) -> Self {
		self.outputs = outputs;
		self
	}

	pub fn with_include_dirs(mut self, include_dirs: Vec<PathBuf>) -> Self {
		self.include_dirs = include_dirs;
		self
	}

	pub fn with_defines(mut self, defines: Vec<(String, Option<String>)>) -> Self {
		self.defines = defines;
		self
	}

	pub fn with_compiler_flags(mut self, flags: Vec<String>) -> Self {
		self.compiler_flags = flags;
		self
	}

	pub fn with_linker_flags(mut self, flags: Vec<String>) -> Self {
		self.linker_flags = flags;
		self
	}

	pub fn with_system_libs(mut self, libs: Vec<String>) -> Self {
		self.system_libs = libs;
		self
	}

	pub fn with_workdir(mut self, workdir: PathBuf) -> Self {
		self.workdir = workdir;
		self
	}

	pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
		self.env = env;
		self
	}

	// -----------------------------------------------------------------------
	// Accessors
	// -----------------------------------------------------------------------

	pub fn output_name(&self) -> String {
		self.outputs
			.first()
			.and_then(|p| p.file_name())
			.map(|n| n.to_string_lossy().to_string())
			.unwrap_or_else(|| self.name.clone())
	}

	pub fn command(&self) -> Option<&str> {
		match &self.component_type {
			ComponentType::Custom { command, .. } => Some(command),
			_ => None,
		}
	}

	pub fn args(&self) -> Option<&[String]> {
		match &self.component_type {
			ComponentType::Custom { args, .. } => Some(args),
			_ => None,
		}
	}

	pub fn inputs(&self) -> &[PathBuf] {
		&self.sources
	}
}
