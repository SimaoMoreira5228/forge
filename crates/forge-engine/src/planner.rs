use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_core::{ActionSpec, BuildGraph, ComponentId, ComponentKind, OutputDeclaration, OutputKind, Platform, Profile};
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::TargetDecl;
use forge_script::cells::{ActionDecl, CellHooks, ComponentView, lower};

use crate::hasher;
use crate::std_cells::StdCells;
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
	pub cells: &'a StdCells,
}

pub type DeclMap = BTreeMap<String, TargetDecl>;

struct Planner<'a> {
	ctx: &'a PlanContext<'a>,
	dag: ActionDag,
	producer_of: BTreeMap<PathBuf, usize>,
	archive_of: BTreeMap<String, PathBuf>,
}

const CELL_LANGUAGES: &[(&str, &[&str])] = &[("c", &["c", "h", "cpp", "cc", "cxx", "C", "hpp", "hh", "hxx"])];

pub fn build_action_dag(ctx: &PlanContext<'_>) -> Result<ActionDag, ForgeDiagnostic> {
	let order = ctx.graph.topological_order().map_err(cycle_error)?;
	let mut planner = Planner {
		ctx,
		dag: ActionDag::default(),
		producer_of: BTreeMap::new(),
		archive_of: BTreeMap::new(),
	};
	for id in order {
		let component = ctx.graph.component(id);
		check_compatibility(component, ctx.platform)?;
		match component.kind {
			ComponentKind::Library { .. } => planner.plan_component(id, "library")?,
			ComponentKind::Binary => planner.plan_component(id, "binary")?,
			ComponentKind::Test => planner.plan_component(id, "test")?,
			ComponentKind::Generic { .. } => planner.plan_rule(id)?,
		}
	}
	Ok(planner.dag)
}

impl<'a> Planner<'a> {
	fn emit(&mut self, mut spec: ActionSpec) -> usize {
		let unique: BTreeSet<PathBuf> = spec.inputs.iter().cloned().collect();
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

	fn plan_rule(&mut self, id: ComponentId) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let decl = self.decl_for(id)?.clone();
		if decl.command.as_deref().is_none_or(str::is_empty) {
			return Err(ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!("rule `{label}` has no command"),
			));
		}
		let command = decl.command.expect("checked above");
		let toolchain_id = decl
			.compiler
			.as_deref()
			.and_then(|name| self.ctx.toolchains.get(name))
			.map(tool_id);
		self.emit(ActionSpec {
			name: format!("run rule {label}"),
			component: label,
			command,
			args: decl.args.clone(),
			inputs: decl.resolved_inputs.clone(),
			outputs: decl
				.outputs
				.iter()
				.map(|(path, is_dir)| OutputDeclaration {
					path: path.clone(),
					kind: if *is_dir { OutputKind::Directory } else { OutputKind::File },
				})
				.collect(),
			env: decl.env.clone(),
			toolchain_id,
		});
		Ok(())
	}

	fn plan_component(&mut self, id: ComponentId, kind: &'static str) -> Result<(), ForgeDiagnostic> {
		let component = self.ctx.graph.component(id);
		let label = component.label.to_string();
		if component.sources.is_empty() {
			return Err(ForgeDiagnostic::error(
				codes::inputs::MISSING_INPUT,
				format!("`{label}` declares no sources"),
			));
		}
		let language = cell_language(&component.sources[0]).ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!(
					"`{label}` declares `{}`, which no std cell handles",
					component.sources[0].display()
				),
			)
			.with_help(format!("supported source types: {}", render_supported_languages()))
		})?;
		let script = self.ctx.cells.get(language)?;

		let decl = self.decl_for(id)?.clone();
		let mut hdrs: Vec<String> = component.headers.iter().map(|p| path_string(p)).collect();
		let mut includes = decl.includes.clone();
		let dep_archives = if kind == "library" {
			Vec::new()
		} else {
			self.transitive_archives(id)
		};
		for &dep in self.ctx.graph.transitive_dependencies(id).iter() {
			hdrs.extend(self.ctx.graph.component(dep).headers.iter().map(|p| path_string(p)));
			if let Ok(dep_decl) = self.decl_for(dep) {
				includes.extend(dep_decl.includes.iter().cloned());
			}
		}

		let profile = self.ctx.profile.clone();
		let pkg_slug = self.package_slug(id);
		let name = component.label.name().to_string();
		let bin_dir = self.tool_for(&decl)?.bin_dir.clone();
		let digest_tool = self.tool_for(&decl)?;
		let tool_digest = tool_id(digest_tool);

		let hooks = CellHooks {
			obj_path: Box::new(move |src| object_path(Path::new(src), &profile).to_string_lossy().into_owned()),
			prior_depfile_headers: Box::new(|obj| {
				previous_depfile_headers(&PathBuf::from(format!("{obj}.d")))
					.into_iter()
					.map(|p| p.to_string_lossy().into_owned())
					.collect()
			}),
			lib_path: Box::new(move |component_name| {
				PathBuf::from(format!("forge-out/lib/{pkg_slug}/lib{component_name}.a"))
					.to_string_lossy()
					.into_owned()
			}),
			bin: Box::new(move |binary_name| {
				let candidate = bin_dir.join(binary_name);
				if candidate.is_file() {
					candidate.to_string_lossy().into_owned()
				} else {
					String::new()
				}
			}),
			tool_id: Box::new(move || tool_digest.clone()),
		};

		let view = ComponentView {
			label,
			name,
			kind: kind.to_string(),
			srcs: component.sources.iter().map(|p| path_string(p)).collect(),
			hdrs,
			includes,
			defines: decl.defines.clone(),
			flags: decl.flags.clone(),
			standard: decl.standard.clone(),
			system_libs: decl.system_libs.clone(),
			run_args: decl.args.clone(),
			data: decl.data.iter().map(|p| path_string(p)).collect(),
			env: decl.env.clone(),
			dep_archives: dep_archives.iter().map(|p: &PathBuf| path_string(p)).collect(),
			linker: match &decl.linker {
				Some(spec) => crate::toolchain::resolve_tool_path(self.ctx.toolchains, spec)?
					.to_string_lossy()
					.into_owned(),
				None => String::new(),
			},
			link_flags: decl.link_flags.clone(),
			platform_os: self.ctx.platform.os.clone(),
			platform_arch: self.ctx.platform.arch.clone(),
			platform_abi: self.ctx.platform.abi.clone().unwrap_or_default(),
			profile: forge_script::cells::ProfileView {
				name: self.ctx.profile.name.clone(),
				opt_level: i64::from(self.ctx.profile.opt_level),
				debug: self.ctx.profile.debug,
				lto: self.ctx.profile.lto,
				strip: self.ctx.profile.strip,
				coverage: self.ctx.profile.coverage,
				defines: self.ctx.profile.defines.clone(),
				sanitizers: self.ctx.profile.sanitizers.clone(),
			},
		};

		let actions = lower(script, &view, hooks)?;
		self.register_cell_actions(actions, id, kind)
	}

	fn register_cell_actions(
		&mut self,
		actions: Vec<ActionDecl>,
		id: ComponentId,
		kind: &'static str,
	) -> Result<(), ForgeDiagnostic> {
		let label = self.ctx.graph.component(id).label.to_string();
		let name = self.ctx.graph.component(id).label.name().to_string();
		let pkg_slug = self.package_slug(id);
		let expected_archive = PathBuf::from(format!("forge-out/lib/{pkg_slug}/lib{name}.a"));
		let mut archive_declared = false;

		for action in actions {
			if kind == "library" && action.outputs.first().map(|(p, _)| PathBuf::from(p)) == Some(expected_archive.clone()) {
				archive_declared = true;
			}
			let spec = ActionSpec {
				name: action.name,
				component: label.clone(),
				command: action.command,
				args: action.args,
				inputs: action.inputs.iter().map(PathBuf::from).collect(),
				outputs: action
					.outputs
					.iter()
					.map(|(path, is_dir)| OutputDeclaration {
						path: PathBuf::from(path),
						kind: if *is_dir { OutputKind::Directory } else { OutputKind::File },
					})
					.collect(),
				env: action.env.clone(),
				toolchain_id: action.toolchain_id.clone(),
			};
			self.emit(spec);
		}

		if kind == "library" {
			if !archive_declared {
				return Err(ForgeDiagnostic::error(
					codes::script::WRONG_TYPE,
					format!(
						"cell did not declare the archive output `{}` for `{label}`",
						expected_archive.display()
					),
				));
			}
			self.archive_of.insert(label, expected_archive);
		}
		Ok(())
	}

	fn transitive_archives(&self, id: ComponentId) -> Vec<PathBuf> {
		self.ctx
			.graph
			.transitive_dependencies(id)
			.iter()
			.filter_map(|&dep| {
				let dep_label = self.ctx.graph.component(dep).label.to_string();
				self.archive_of.get(&dep_label).cloned()
			})
			.collect()
	}
}

fn cell_language(source: &Path) -> Option<&'static str> {
	let extension = source.extension()?.to_str()?;
	CELL_LANGUAGES
		.iter()
		.find(|(_, extensions)| extensions.contains(&extension))
		.map(|(language, _)| *language)
}

fn render_supported_languages() -> String {
	CELL_LANGUAGES
		.iter()
		.map(|(lang, extensions)| format!("{} ({})", lang, extensions.join(", ")))
		.collect::<Vec<_>>()
		.join(", ")
}

fn path_string(path: &Path) -> String {
	path.to_string_lossy().into_owned()
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
		.map(|cycle| cycle.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" -> "))
		.collect();
	ForgeDiagnostic::error(
		codes::graph::CYCLE_DETECTED,
		format!("dependency cycle detected: {}", rendered.join("; ")),
	)
}

fn tool_id(tool: &ResolvedToolchain) -> String {
	format!("{}@{}", tool.name, &tool.digest[..12.min(tool.digest.len())])
}

fn object_path(source: &Path, profile: &Profile) -> PathBuf {
	let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("src");
	let suffix = hasher::hex(blake3::hash(source.to_string_lossy().as_bytes()).as_bytes())[..8].to_string();
	PathBuf::from(format!("forge-out/obj/{}/{}_{suffix}.o", profile.name, stem))
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
		.filter(|token| *token != "\\")
		.filter_map(|token| {
			let path = PathBuf::from(token.replace('\\', ""));
			let is_header = path
				.extension()
				.and_then(|e| e.to_str())
				.map(|e| HEADER_EXTS.contains(&e))
				.unwrap_or(false);
			(is_header && path.is_file()).then_some(path)
		})
		.collect()
}
