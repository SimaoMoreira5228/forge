use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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
}

impl Default for ComponentType {
	fn default() -> Self {
		ComponentType::Library {
			link_type: LinkType::Static,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
	pub id: ComponentId,
	pub name: String,
	pub component_type: ComponentType,
	pub target_name: String,
	pub sources: Vec<PathBuf>,
	pub outputs: Vec<PathBuf>,
	pub include_dirs: Vec<PathBuf>,
	pub defines: Vec<(String, Option<String>)>,
	pub compiler_flags: Vec<String>,
	pub linker_flags: Vec<String>,
	pub system_libs: Vec<String>,
	pub workdir: PathBuf,
	pub env: HashMap<String, String>,
}

impl Component {
	pub fn new(name: impl Into<String>, target_name: impl Into<String>) -> Self {
		Self {
			id: ComponentId::new(),
			name: name.into(),
			component_type: ComponentType::default(),
			target_name: target_name.into(),
			sources: Vec::new(),
			outputs: Vec::new(),
			include_dirs: Vec::new(),
			defines: Vec::new(),
			compiler_flags: Vec::new(),
			linker_flags: Vec::new(),
			system_libs: Vec::new(),
			workdir: std::env::current_dir().unwrap_or_default(),
			env: HashMap::new(),
		}
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

	pub fn custom(name: impl Into<String>, target_name: impl Into<String>, command: String, args: Vec<String>) -> Self {
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
