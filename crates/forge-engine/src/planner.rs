use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::{ActionSpec, BuildGraph, ComponentId, ComponentKind, OutputDeclaration, OutputKind, Platform, Profile};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::TargetDecl;

use crate::hasher;
use crate::toolchain::ResolvedToolchain;

#[derive(Debug, Default)]
pub struct ActionDag {
	pub specs: Vec<ActionSpec>,
	pub deps: Vec<Vec<usize>>,
}

impl ActionDag {
	pub fn dependents(&self) -> Vec<Vec<usize>> {
		let mut rev = vec![Vec::new(); self.specs.len()];
		for (i, ds) in self.deps.iter().enumerate() {
			for &d in ds {
				rev[d].push(i);
			}
		}
		rev
	}

	pub fn tests(&self) -> Vec<usize> {
		self.specs
			.iter()
			.enumerate()
			.filter(|(_, s)| s.name.starts_with("run "))
			.map(|(i, _)| i)
			.collect()
	}
}

pub struct PlanContext<'a> {
	pub graph: &'a BuildGraph,
	pub decls: &'a BTreeMap<String, TargetDecl>,
	pub profile: &'a Profile,
	pub platform: &'a Platform,
	pub toolchains: &'a BTreeMap<String, ResolvedToolchain>,
}

pub type DeclMap = BTreeMap<String, TargetDecl>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
	C,
	Cpp,
}

impl Language {
	fn of(path: &Path) -> Option<Self> {
		match path.extension()?.to_str()? {
			"c" => Some(Language::C),
			"cpp" | "cc" | "cxx" | "C" => Some(Language::Cpp),
			_ => None,
		}
	}

	fn compiler_names(self) -> &'static [&'static str] {
		match self {
			Language::C => &["clang", "gcc", "cc"],
			Language::Cpp => &["clang++", "g++", "c++"],
		}
	}

	fn default_standard(self) -> &'static str {
		match self {
			Language::C => "c17",
			Language::Cpp => "c++20",
		}
	}
}

struct Planner<'a> {
	ctx: &'a PlanContext<'a>,
	dag: ActionDag,
	producer_of: BTreeMap<PathBuf, usize>,
	archive_of: BTreeMap<String, PathBuf>,
}

pub fn build_action_dag(ctx: &PlanContext<'_>) -> Result<ActionDag, ForgeDiagnostic> {
	let order = ctx.graph.topological_order().map_err(cycle_error)?;
	let mut planner = Planner {
		ctx,
		dag: ActionDag::default(),
		producer_of: BTreeMap::new(),
		archive_of: BTreeMap::new(),
	};
	for id in order {
		let kind = ctx.graph.component(id).kind.clone();
		check_compatibility(ctx.graph.component(id), ctx.platform)?;
		match kind {
			ComponentKind::Library { .. } => planner.plan_library(id)?,
			ComponentKind::Binary => planner.plan_binary_or_test(id, false)?,
			ComponentKind::Test => planner.plan_binary_or_test(id, true)?,
			ComponentKind::Generic { .. } => {}
		}
	}
	Ok(planner.dag)
}

impl<'a> Planner<'a> {
	fn emit(&mut self, mut spec: ActionSpec) -> usize {
		let unique: std::collections::BTreeSet<PathBuf> = spec.inputs.iter().cloned().collect();
		spec.inputs = unique.into_iter().collect();

		let index = self.dag.specs.len();
		let mut deps = Vec::new();
		for input in &spec.inputs {
			if let Some(&producer) = self.producer_of.get(input) {
				deps.push(producer);
			}
		}
		for output in &spec.outputs {
			self.producer_of.insert(output.path.clone(), index);
		}
		self.dag.specs.push(spec);
		self.dag.deps.push(deps);
		index
	}

	fn decl_for(&self, id: ComponentId) -> Result<&TargetDecl, ForgeDiagnostic> {
		let key = self.ctx.graph.component(id).label.to_string();
		self.ctx.decls.get(&key).ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::targets::UNKNOWN_TARGET,
				format!("no declaration found for component `{key}`"),
			)
		})
	}

	fn package_slug(&self, id: ComponentId) -> String {
		self.ctx.graph.component(id).label.package().replace('/', "_")
	}

	fn plan_library(&mut self, id: ComponentId) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let objects = self.compile_sources(id)?;
		let decl = self.decl_for(id)?.clone();
		let pkg = self.package_slug(id);
		let archive = PathBuf::from(format!("forge-out/lib/{pkg}/lib{}.a", decl.name));
		let tool = self.tool_for(&decl)?;
		let ar = tool.binary("ar").ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("toolchain `{}` provides no `ar`; cannot archive `{}`", tool.name, decl.name),
			)
		})?;
		let inputs = objects.clone();
		let ar_path = ar.to_string_lossy().into_owned();
		let archive_id = tool_id(tool);
		self.emit(ActionSpec {
			name: format!("archive {label}"),
			component: label.clone(),
			command: ar_path,
			args: ["rcs".into(), archive.to_string_lossy().into_owned()]
				.into_iter()
				.chain(objects.iter().map(|p| p.to_string_lossy().into_owned()))
				.collect(),
			inputs,
			outputs: vec![file_output(&archive)],
			env: Default::default(),
			toolchain_id: Some(archive_id),
		});
		self.archive_of.insert(label, archive);
		Ok(())
	}

	fn plan_binary_or_test(&mut self, id: ComponentId, is_test: bool) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let objects = self.compile_sources(id)?;
		let decl = self.decl_for(id)?.clone();

		let dep_archives = self.transitive_archives(id);
		let kind_dir = if is_test { "test" } else { "bin" };
		let output = PathBuf::from(format!("forge-out/{kind_dir}/{}/{}", self.ctx.profile.name, decl.name));

		let tool = self.tool_for(&decl)?;
		let linker = linker_binary(tool)?;
		// Headers never feed ar/ld; only compile actions consume them.
		let mut link_inputs = objects.clone();
		link_inputs.extend(dep_archives.iter().cloned());

		let mut args: Vec<String> = objects
			.iter()
			.chain(dep_archives.iter())
			.map(|p| p.to_string_lossy().into_owned())
			.collect();
		args.extend(profile_link_flags(self.ctx.profile));
		for lib in &decl.system_libs {
			args.push("-l".into());
			args.push(lib.clone());
		}
		args.push("-o".into());
		args.push(output.to_string_lossy().into_owned());

		let linker_path = linker.to_string_lossy().into_owned();
		let link_tool = tool_id(tool);
		self.emit(ActionSpec {
			name: format!("link {label}"),
			component: label.clone(),
			command: linker_path,
			args,
			inputs: link_inputs,
			outputs: vec![file_output(&output)],
			env: Default::default(),
			toolchain_id: Some(link_tool),
		});

		if is_test {
			let exe_path = output.clone();
			let mut inputs = vec![exe_path];
			inputs.extend(decl.data.iter().cloned());
			self.emit(ActionSpec {
				name: format!("run {label}"),
				component: label,
				command: output.to_string_lossy().into_owned(),
				args: decl.args.clone(),
				inputs,
				outputs: vec![],
				env: decl.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
				toolchain_id: None,
			});
		}
		Ok(())
	}

	fn compile_sources(&mut self, id: ComponentId) -> Result<Vec<PathBuf>, ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let decl = self.decl_for(id)?.clone();

		let sources = self.ctx.graph.component(id).sources.clone();
		if sources.is_empty() && matches!(self.ctx.graph.component(id).kind, ComponentKind::Library { .. }) {
			return Err(ForgeDiagnostic::error(
				codes::inputs::MISSING_INPUT,
				format!("library `{label}` declares no sources"),
			));
		}
		let tool = self.tool_for(&decl)?;
		let tool_bin = tool.bin_dir.clone();
		let compile_tool_id = tool_id(tool);
		let tool_name = tool.name.clone();
		let mut objects = Vec::new();
		let mut declared_headers: Vec<PathBuf> = self.ctx.graph.component(id).headers.clone();
		let mut inherited_includes: Vec<String> = Vec::new();
		let own_decl = self.decl_for(id)?.clone();
		for &dep in self.ctx.graph.transitive_dependencies(id).iter() {
			declared_headers.extend(self.ctx.graph.component(dep).headers.iter().cloned());
			if let Ok(dep_decl) = self.decl_for(dep) {
				inherited_includes.extend(dep_decl.includes.iter().cloned());
			}
		}
		let _ = &own_decl;
		for source in sources {
			let Some(lang) = Language::of(&source) else {
				return Err(ForgeDiagnostic::error(
					codes::script::WRONG_TYPE,
					format!(
						"`{label}` declares `{}`, which no built-in language handles",
						source.display()
					),
				)
				.with_help("declare it as a `rule` instead"));
			};
			let compiler = lang
				.compiler_names()
				.iter()
				.map(|n| tool_bin.join(n))
				.find(|p| p.is_file())
				.ok_or_else(|| {
					ForgeDiagnostic::error(
						codes::hermetic::TOOLCHAIN_MISMATCH,
						format!(
							"toolchain `{}` provides none of {}",
							tool_name,
							lang.compiler_names().join(", ")
						),
					)
				})?;

			let obj = object_path(&source, self.ctx.profile);
			let depfile = PathBuf::from(format!("{}.d", obj.to_string_lossy()));

			let mut args = profile_compile_args(self.ctx.profile);
			args.push(format!(
				"-std={}",
				decl.standard.as_deref().unwrap_or(lang.default_standard())
			));
			for include in decl.includes.iter().chain(inherited_includes.iter()) {
				args.push(format!("-I{}", include));
			}
			for define in &decl.defines {
				args.push(format!("-D{}", define));
			}
			args.extend(decl.flags.iter().cloned());
			args.push("-MMD".into());
			args.push("-MF".into());
			args.push(depfile.to_string_lossy().into_owned());
			args.push("-c".into());
			args.push(source.to_string_lossy().into_owned());
			args.push("-o".into());
			args.push(obj.to_string_lossy().into_owned());
			let mut inputs = vec![source.clone()];
			inputs.extend(declared_headers.iter().cloned());
			inputs.extend(previous_depfile_headers(&depfile));

			let compiler_path = compiler.to_string_lossy().into_owned();
			self.emit(ActionSpec {
				name: format!("compile {}", source.display()),
				component: label.clone(),
				command: compiler_path,
				args,
				inputs,
				outputs: vec![file_output(&obj), file_output(&depfile)],
				env: decl.env.clone(),
				toolchain_id: Some(compile_tool_id.clone()),
			});
			objects.push(obj);
		}
		Ok(objects)
	}

	fn tool_for(&self, decl: &TargetDecl) -> Result<&ResolvedToolchain, ForgeDiagnostic> {
		let mut candidates: Vec<&str> = vec!["clang", "gcc", "cc"];
		if let Some(name) = &decl.compiler {
			candidates.insert(0, name.as_str());
		}
		for name in candidates {
			if let Some(tool) = self.ctx.toolchains.get(name) {
				return Ok(tool);
			}
		}
		Err(ForgeDiagnostic::error(
			codes::hermetic::TOOLCHAIN_MISMATCH,
			format!(
				"component `{}` needs compiler {:?} but FORGE_ROOT configures: {}",
				decl.name,
				decl.compiler,
				self.ctx.toolchains.keys().cloned().collect::<Vec<_>>().join(", ")
			),
		)
		.with_help("add a [toolchains.<name>] section to FORGE_ROOT"))
	}

	fn transitive_archives(&self, id: ComponentId) -> Vec<PathBuf> {
		self.ctx
			.graph
			.transitive_dependencies(id)
			.iter()
			.filter_map(|&dep| {
				let label = self.ctx.graph.component(dep).label.to_string();
				self.archive_of.get(&label).cloned()
			})
			.collect()
	}
}

fn linker_binary(tool: &ResolvedToolchain) -> Result<PathBuf, ForgeDiagnostic> {
	["clang++", "g++", "c++", "clang", "gcc", "cc"]
		.iter()
		.find_map(|n| tool.binary(n))
		.ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("toolchain `{}` provides no linker", tool.name),
			)
		})
}

fn check_compatibility(component: &forge_core::Component, platform: &Platform) -> Result<(), ForgeDiagnostic> {
	if component.is_compatible_with(platform) {
		return Ok(());
	}
	Err(ForgeDiagnostic::error(
		codes::platform::CONSTRAINT_VIOLATION,
		format!(
			"`{}` is incompatible with platform {}",
			component.label,
			platform.catalog_key()
		),
	)
	.with_help(format!("requires {}", component.compatible_with.join(", "))))
}

fn cycle_error(cycles: Vec<Vec<forge_core::Label>>) -> ForgeDiagnostic {
	let rendered: Vec<String> = cycles
		.iter()
		.map(|c| c.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" -> "))
		.collect();
	ForgeDiagnostic::error(
		codes::graph::CYCLE_DETECTED,
		format!("dependency cycle detected: {}", rendered.join("; ")),
	)
}

fn file_output(path: &Path) -> OutputDeclaration {
	OutputDeclaration {
		path: path.to_path_buf(),
		kind: OutputKind::File,
	}
}

fn tool_id(tool: &ResolvedToolchain) -> String {
	format!("{}@{}", tool.name, &tool.digest[..12.min(tool.digest.len())])
}

fn object_path(source: &Path, profile: &Profile) -> PathBuf {
	let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("src");
	let suffix = hasher::hex(blake3::hash(source.to_string_lossy().as_bytes()).as_bytes())[..8].to_string();
	PathBuf::from(format!("forge-out/obj/{}/{}_{suffix}.o", profile.name, stem))
}

fn profile_compile_args(profile: &Profile) -> Vec<String> {
	let mut args = vec![format!("-O{}", profile.opt_level)];
	if profile.debug {
		args.push("-g".into());
	}
	args.extend(profile_link_flags(profile));
	for define in &profile.defines {
		args.push(format!("-D{define}"));
	}
	args
}

fn profile_link_flags(profile: &Profile) -> Vec<String> {
	let mut args = Vec::new();
	for sanitizer in &profile.sanitizers {
		args.push(format!("-fsanitize={sanitizer}"));
	}
	if profile.coverage {
		args.push("--coverage".into());
	}
	if profile.lto {
		args.push("-flto".into());
	}
	args
}

fn previous_depfile_headers(depfile: &Path) -> Vec<PathBuf> {
	let Ok(text) = std::fs::read_to_string(depfile) else {
		return Vec::new();
	};
	parse_depfile(&text)
}

fn parse_depfile(text: &str) -> Vec<PathBuf> {
	const HEADER_EXTS: &[&str] = &["h", "hh", "hpp", "hxx", "inc", "inl"];
	let flattened = text.replace("\\\n", " ");
	let Some((_, rest)) = flattened.split_once(':') else {
		return Vec::new();
	};
	rest.split_whitespace()
		.filter(|t| *t != "\\")
		.filter_map(|t| {
			let path = PathBuf::from(t.replace('\\', ""));
			let header = path
				.extension()
				.and_then(|e| e.to_str())
				.map(|e| HEADER_EXTS.contains(&e))
				.unwrap_or(false);
			(header && path.is_file()).then_some(path)
		})
		.collect()
}
