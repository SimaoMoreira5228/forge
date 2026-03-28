use crate::{
	cache::{BuildCache, CacheDb},
	config::Config,
	error::ForgeError,
	forge_root_config::ForgeRootConfig,
	hermetic::{ActionSpec, HermeticPolicy, SandboxRunner},
	lua_api,
	graph::BuildGraph,
	profile::{BuildProfile, resolver::ProfileResolver},
};
use serde::{Deserialize, Serialize};
use anyhow::Context;
use blake3::Hasher;
use dashmap::DashMap;
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use mlua::{Lua, UserData, ObjectLike, LuaSerdeExt};
use rayon::prelude::*;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::{
	borrow::Cow,
	collections::HashMap,
	path::{Path, PathBuf},
	sync::{Arc, Mutex},
	time::{Instant, Duration},
};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
	pub name: String,
	pub command: String,
	pub args: Vec<String>,
	pub env: HashMap<String, String>,
	pub inputs: Vec<String>,
	pub outputs: Vec<String>,
	pub dependencies: Vec<String>,
	pub workdir: PathBuf,
	/// If set, this rule should be built for the given configuration (host/exec/target)
	/// rather than the default target configuration.
	pub exec_cfg: Option<crate::graph::ConfigTransition>,
}

impl UserData for Rule {}

#[derive(Clone)]
pub struct Project {
	pub path: PathBuf,
	pub config: Config,
	pub forge_root_config: ForgeRootConfig,
	pub build_graph: Arc<DashMap<String, Rule>>,
	pub dependency_graph: Arc<Mutex<BuildGraph>>,
	pub output_map: Arc<DashMap<String, String>>,
	pub cache: BuildCache,
	pub build_profile: BuildProfile,
	cas_path: PathBuf,
	pub lua: Lua,
	hermetic_policy: HermeticPolicy,
	pub explain_trace: Arc<DashMap<String, Vec<String>>>,
	pub platform_registry: Arc<crate::platform::PlatformRegistry>,
}

impl Project {
	fn process_rule_inputs<'a>(&'a self, rule: &'a Rule) -> Result<Vec<Cow<'a, str>>, ForgeError> {
		let mut processed_inputs = Vec::new();
		for input in &rule.inputs {
			if self.path.join(input).exists() {
				processed_inputs.push(Cow::Borrowed(input.as_str()));
			} else {
				processed_inputs.push(Cow::Owned(input.clone()));
			}
		}
		Ok(processed_inputs)
	}

	pub fn new(
		path: PathBuf,
		config: Config,
		cli_mode: Option<crate::hermetic::PolicyMode>,
		trace_access: bool,
		why_non_hermetic: bool,
	) -> Result<Self, ForgeError> {
		let forge_root_path = path.join("FORGE_ROOT");
		let forge_root_config = ForgeRootConfig::load(&forge_root_path).map_err(|_| ForgeError::ForgeRootNotFound {
			path: forge_root_path.display().to_string(),
		})?;

		let mut hermetic_policy = crate::hermetic::HermeticPolicy::new(cli_mode.unwrap_or(crate::hermetic::PolicyMode::Strict));
		if cli_mode.is_none() {
			if let Some(ref root_hermetic) = forge_root_config.project.hermetic {
				let mode_res: Result<crate::hermetic::PolicyMode, _> = root_hermetic.parse();
				if let Ok(mode) = mode_res {
					hermetic_policy.mode = mode;
				}
			}
		}
		hermetic_policy.trace_access = trace_access;
		hermetic_policy.why_non_hermetic = why_non_hermetic;

		let profile_name = config.profile.as_deref().unwrap_or("debug");
		let resolver = ProfileResolver::new(&forge_root_config.profile);
		let build_profile = resolver.resolve(profile_name).map_err(|e| anyhow::anyhow!("Failed to resolve profile '{}': {}", profile_name, e))?;

		let output_dir = path.join(&forge_root_config.build.cache_dir);
		let cas_path = output_dir.join("cas");
		std::fs::create_dir_all(&output_dir)?;
		std::fs::create_dir_all(&cas_path)?;

		// Find prelude directory
		let mut prelude_found = false;
		
		// 1. Check paths from FORGE_ROOT
		for prelude_pattern in &forge_root_config.discovery.prelude_paths {
			let cand = path.join(prelude_pattern);
			if cand.exists() {
				prelude_found = true;
				break;
			}
		}

		// 2. Check default "prelude" folder
		if !prelude_found {
			let cand = path.join("prelude");
			if cand.exists() {
				prelude_found = true;
			}
		}

		// 3. Try parent directories (repo case)
		if !prelude_found {
			let mut current = path.as_path();
			while let Some(parent) = current.parent() {
				let candidate = parent.join("prelude");
				if candidate.exists() {
					prelude_found = true;
					break;
				}
				current = parent;
			}
		}

		if !prelude_found {
			return Err(ForgeError::PreludeNotFound(path.join("prelude").display().to_string()));
		}

		let cache_path = output_dir.join("cache.json");
		let cache = BuildCache::load(&cache_path);

		cache.validate_and_clean(&path);

		// Auto-migrate old JSON cache to SQLite if present
		let db_path = cas_path.join("cache.db");
		if cache_path.exists() {
			match CacheDb::new(&db_path) {
				Ok(db) => match db.migrate_from_json(&cache_path) {
					Ok(count) => {
						if count > 0 {
							log::info!("Migrated {} cache entries to SQLite.", count);
							let _ = db.delete_old_cache(&cache_path);
						}
					}
					Err(e) => {
						log::warn!("Cache migration failed: {}", e);
					}
				},
				Err(e) => {
					log::warn!("Failed to open SQLite cache: {}", e);
				}
			}
		} else {
			// Initialize empty SQLite cache for new builds
			let _ = CacheDb::new(&db_path);
		}

		let platform_registry = {
			let mut registry = crate::platform::PlatformRegistry::new();
			for (name, config) in &forge_root_config.platforms {
				registry.register(name, config);
			}
			Arc::new(registry)
		};

		Ok(Self {
			path,
			config,
			forge_root_config,
			build_graph: Arc::new(DashMap::new()),
			dependency_graph: Arc::new(Mutex::new(BuildGraph::new())),
			output_map: Arc::new(DashMap::new()),
			cache,
			build_profile,
			cas_path,
			lua: Lua::new(),
			hermetic_policy,
			explain_trace: Arc::new(DashMap::new()),
			platform_registry,
		})
	}

	pub fn reset_graph(&self) {
		self.build_graph.clear();
		self.output_map.clear();
		let mut graph = self.dependency_graph.lock().unwrap();
		*graph = BuildGraph::new();
	}


	fn setup_lua_environment(&self) -> Result<(), ForgeError> {
		lua_api::init::setup_lua_environment(&self.lua, self)?;
		Ok(())
	}

	pub fn get_toolchain_paths(&self) -> Vec<PathBuf> {
		let mut paths = Vec::new();
		let toolchain_cache = self.path.join(".forge").join("toolchains");

		log::debug!("Looking for toolchains in: {}", toolchain_cache.display());

		for (_name, config) in &self.forge_root_config.toolchain {
			if config.from == "path" {
				if let Some(ref path_str) = config.path {
					let candidate = PathBuf::from(path_str).join("bin");
					if candidate.exists() {
						paths.push(candidate);
					}
				}
			}
		}

		if toolchain_cache.exists() {
			for entry in WalkDir::new(&toolchain_cache).into_iter().flatten() {
				let p = entry.path();
				if p.is_dir() && p.file_name().map(|n| n == "bin").unwrap_or(false) {
					paths.push(p.to_path_buf());
				}
			}
		}

		paths.sort();
		paths.dedup();

		log::debug!("Found toolchain paths: {:?}", paths);
		paths
	}

	fn calculate_toolchain_fingerprint(&self) -> Result<String, ForgeError> {
		let mut hasher = Hasher::new();
		let toolchain_cache = self.path.join(".forge").join("toolchains");

		for (name, config) in &self.forge_root_config.toolchain {
			hasher.update(name.as_bytes());
			hasher.update(config.from.as_bytes());
			if let Some(ref version) = config.version {
				hasher.update(version.as_bytes());
			}
			if let Some(ref url) = config.url {
				hasher.update(url.as_bytes());
			}
			if let Some(ref path_str) = config.path {
				hasher.update(path_str.as_bytes());
			}

			let source = match config.from.as_str() {
				"path" => "path",
				"url" => "url",
				"auto" => "auto",
				_ => "version",
			};

			if source == "path" {
				if let Some(ref path_str) = config.path {
					let bin_dir = PathBuf::from(path_str).join("bin");
					if bin_dir.exists() {
						if let Ok(entries) = std::fs::read_dir(&bin_dir) {
							for entry in entries.flatten() {
								if let Ok(meta) = entry.metadata() {
									if meta.is_file() {
										hasher.update(entry.file_name().to_string_lossy().as_bytes());
									}
								}
							}
						}
					}
				}
			} else if source == "version" || source == "url" {
				let version = config.version.as_deref().unwrap_or("latest");
				let toolchain_dir = toolchain_cache.join(name).join(version);
				let bin_dir = toolchain_dir.join("bin");
				if bin_dir.exists() {
					if let Ok(entries) = std::fs::read_dir(&bin_dir) {
						for entry in entries.flatten() {
							if let Ok(meta) = entry.metadata() {
								if meta.is_file() {
									hasher.update(entry.file_name().to_string_lossy().as_bytes());
								}
							}
						}
					}
				}
			}
		}

		Ok(hasher.finalize().to_hex().to_string())
	}

	pub fn load_graph(&self) -> Result<(), ForgeError> {
		self.reset_graph();
		self.setup_lua_environment()?;

		let forge_files = self.find_forge_files(&self.path)?;

		log::debug!("Found {} FORGE files to load: {:?}", forge_files.len(), forge_files);
		for forge_file in &forge_files {
			log::debug!("Loading FORGE file: {}", forge_file.display());
			
			// Determine package name based on directory relative to project root
			let rel_dir = forge_file.parent()
				.and_then(|p| p.strip_prefix(&self.path).ok())
				.map(|p| p.to_string_lossy().to_string())
				.unwrap_or_else(|| String::from("."));
			
			// Set current package in graph API
			// We can use the forge table already in globals
			let globals = self.lua.globals();
			if let Ok(forge) = globals.get::<mlua::Table>("forge") {
				if let Ok(graph) = forge.get::<mlua::Table>("graph") {
					let set_package: mlua::Function = graph.get("set_package")?;
					set_package.call::<()>(rel_dir)?;
				}
			}

			let content = std::fs::read_to_string(forge_file)?;

			if content.trim().is_empty() {
				return Err(ForgeError::InvalidForgeFile {
					file: forge_file.display().to_string(),
					error: "FORGE file is empty".to_string(),
					suggestion: "Add build rules to your FORGE file".to_string(),
				});
			}

			if !content.contains("rule") && !content.contains("require") && !content.contains("forge.graph") && !content.contains("graph") {
				return Err(ForgeError::InvalidForgeFile {
					file: forge_file.display().to_string(),
					error: "No build rules found".to_string(),
					suggestion: "Add at least one rule() call to define build steps".to_string(),
				});
			}

			if let Err(e) = self.lua.load(&content).exec() {
				return Err(ForgeError::LuaError {
					file: forge_file.display().to_string(),
					error: e,
				});
			}
		}
		
		// Resolve all deferred dependencies and enforce constraints
		{
			let mut graph = self.dependency_graph.lock().unwrap();
			if let Err(errors) = graph.resolve_all_dependencies(Some(&self.platform_registry)) {
				if let Some(first) = errors.first() {
					return Err(ForgeError::GraphError(first.clone()));
				}
			}
		}

		self.compile_dependency_graph()?;

		Ok(())
	}

	fn compile_dependency_graph(&self) -> Result<(), ForgeError> {
		let dep_graph = self.dependency_graph.lock().unwrap();
		println!("DEBUG: Compiling dependency graph with {} components", dep_graph.component_count());
		for id in dep_graph.component_ids() {
			let comp = match dep_graph.get_component(id) {
				Some(c) => c,
				None => continue,
			};
			log::debug!("Processing component: {} (type: {:?})", comp.name, comp.component_type.kind_name());

			// Create a unique name for the rule (include target)
			let rule_name = format!("{}:{}", comp.name, comp.target_name);

			// For now, we only handle Custom components (Mode 1)
			if let crate::graph::ComponentType::Custom { command, args } = &comp.component_type {
				// Resolve dependencies to rule names and collect their outputs as our inputs
				let mut rule_deps = Vec::new();
				let mut extra_inputs = Vec::new();

				for (dep_ref, _edge) in &comp.dependencies {
					let dep_name = dep_ref.name();
					let dep_target = dep_ref.target().unwrap_or(&comp.target_name);

					if let Some(dep_comp) = dep_graph.get_component_by_name(dep_name, dep_target) {
						let full_name = format!("{}:{}", dep_comp.name, dep_comp.target_name);
						println!("@@@ DEBUG: Found component {} -> rule {}", dep_name, full_name);
						rule_deps.push(full_name);
						// Automatically add dependency outputs to our inputs
						for output in &dep_comp.outputs {
							extra_inputs.push(output.to_string_lossy().to_string());
						}
					} else {
						println!("@@@ DEBUG: Component NOT FOUND: name={}, target={}", dep_name, dep_target);
						// Fallback to literal name if not found in graph
						rule_deps.push(dep_name.to_string());
					}
				}

				let mut rule_inputs = comp.sources.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>();
				rule_inputs.extend(extra_inputs);

				let rule = Rule {
					name: rule_name.clone(),
					command: command.clone(),
					args: args.clone(),
					inputs: rule_inputs,
					outputs: comp.outputs.iter().map(|p| p.to_string_lossy().to_string()).collect(),
					env: comp.env.clone(),
					workdir: comp.workdir.clone(),
					dependencies: rule_deps,
					exec_cfg: comp.exec_cfg.clone(),
				};
				self.build_graph.insert(rule_name, rule);
			}
		}
		Ok(())
	}

	pub fn run(&mut self) -> Result<(), ForgeError> {
		self.load_graph()?;
		self.execute_build_graph()?;

		if self.config.test_mode {
			self.execute_tests()?;
		}

		let cache_path = self.path.join("forge-out").join("cache.json");
		self.cache.save(&cache_path).context("Failed to save build cache")?;

		Ok(())
	}

	pub fn run_coverage(&mut self, output_format: Option<&str>) -> Result<(), ForgeError> {
		self.load_graph()?;
		self.execute_build_graph()?;

		if self.config.test_mode {
			self.execute_tests()?;
		}

		crate::coverage::aggregate_coverage(self, output_format)?;

		let cache_path = self.path.join("forge-out").join("cache.json");
		self.cache.save(&cache_path).context("Failed to save build cache")?;

		Ok(())
	}

	pub fn execute_tests(&self) -> Result<(), ForgeError> {
		use crate::graph::ComponentType;
		let dep_graph = self.dependency_graph.lock().unwrap();
		let mut tests_found = false;
		let mut test_errors = false;

		log::info!("Starting test execution cycle...");

		for id in dep_graph.component_ids() {
			let comp = match dep_graph.get_component(id) {
				Some(c) => c,
				None => continue,
			};

			if let ComponentType::Test {
				executable,
				command,
				args,
				env,
				timeout_secs,
				tags,
				..
			} = &comp.component_type
			{
				// Filter by target
				if !self.config.target_filters.is_empty()
					&& !self.config.target_filters.iter().any(|f| comp.target_name == *f)
				{
					continue;
				}

				// Filter by component name
				if !self.config.component_filters.is_empty()
					&& !self.config.component_filters.iter().any(|f| comp.name.contains(f))
				{
					continue;
				}

				if tags.contains(&"manual".to_string()) {
					log::info!("Skipping manual test: {}", comp.name);
					continue;
				}

				tests_found = true;
				println!("\n=== Running test: {} ===", comp.name);

				let runner = crate::hermetic::SandboxRunner::new(
					self.hermetic_policy.clone(),
					self.path.join(".forge").join("runfiles").join(&comp.name),
				);

				let result = if let Some(exec_path) = executable {
					let exec = if exec_path.is_absolute() {
						exec_path.clone()
					} else {
						self.path.join(exec_path)
					};

					if exec.exists() {
						let action_spec = crate::hermetic::ActionSpec::new(&comp.name)
							.with_command(exec.to_string_lossy().to_string())
							.with_args(args.clone())
							.with_env(env.clone())
							.with_workdir(self.path.clone())
							.with_timeout(*timeout_secs);

						runner.execute(&action_spec).map_err(|e| ForgeError::BuildFailed {
							rule: comp.name.clone(),
							error: format!("Sandbox error: {}", e),
						})
					} else {
						Err(ForgeError::BuildFailed {
							rule: comp.name.clone(),
							error: format!("Test executable not found: {}", exec.display()),
						})
					}
				} else if let Some(cmd_parts) = command {
					if cmd_parts.is_empty() {
						Err(ForgeError::BuildFailed {
							rule: comp.name.clone(),
							error: "Test has empty command list".to_string(),
						})
					} else {
						let mut final_args = cmd_parts[1..].to_vec();
						final_args.extend(args.clone());

						let action_spec = crate::hermetic::ActionSpec::new(&comp.name)
							.with_command(&cmd_parts[0])
							.with_args(final_args)
							.with_env(env.clone())
							.with_workdir(self.path.clone())
							.with_timeout(*timeout_secs);

						runner.execute(&action_spec).map_err(|e| ForgeError::BuildFailed {
							rule: comp.name.clone(),
							error: format!("Sandbox error: {}", e),
						})
					}
				} else {
					Err(ForgeError::BuildFailed {
						rule: comp.name.clone(),
						error: "Test has no binary or command".to_string(),
					})
				};

				// If in coverage mode, extract profiles even if test failed (often useful)
				if self.build_profile.name == "coverage" {
					let action_spec = crate::hermetic::ActionSpec::new(&comp.name); // Dummy spec for path resolution
					let coverage_dir = self.path.join("forge-out").join("coverage");
					if let Err(e) = runner.extract_coverage_data(&action_spec, &coverage_dir) {
						log::warn!("Failed to extract coverage data for {}: {}", comp.name, e);
					}
				}

				match &result {
					Ok(res) => {
						print!("{}", res.stdout);
						eprint!("{}", res.stderr);

						if res.timed_out {
							println!("FAILED: Test '{}' timed out after {:?}", comp.name, res.duration);
							test_errors = true;
						} else if res.success {
							println!("PASSED: {} (in {:?})", comp.name, res.duration);
						} else {
							println!("FAILED: Test '{}' failed with code {:?}", comp.name, res.exit_code);
							test_errors = true;
						}
					}
					Err(e) => {
						println!("{}", e);
						test_errors = true;
					}
				}

				if let Err(ref e) = result {
					eprintln!("FAILED: {}", e);
					test_errors = true;
				}
			}
		}

		if !tests_found && (!self.config.target_filters.is_empty() || !self.config.component_filters.is_empty()) {
			log::warn!("No tests found matching active filters.");
		}

		if test_errors {
			Err(ForgeError::BuildFailed {
				rule: "test_suite".to_string(),
				error: "One or more tests failed".to_string(),
			})
		} else {
			Ok(())
		}
	}

	fn find_forge_files(&self, path: &Path) -> Result<Vec<PathBuf>, ForgeError> {
		let mut forge_files = Vec::new();
		let discovery_config = &self.forge_root_config.discovery;

		for include_pattern in &discovery_config.include {
			let search_path = if include_pattern == "." {
				path.to_path_buf()
			} else {
				path.join(include_pattern)
			};

			if !search_path.exists() {
				log::debug!("Skipping non-existent include path: {}", search_path.display());
				continue;
			}

			let files = if discovery_config.use_gitignore {
				self.find_forge_files_with_gitignore(&search_path, discovery_config)?
			} else {
				self.find_forge_files_simple(&search_path, discovery_config)?
			};

			forge_files.extend(files);
		}

		if forge_files.is_empty() {
			let searched_paths = discovery_config.include.join(", ");
			return Err(ForgeError::NoForgeFilesFound { searched_paths });
		}

		forge_files.sort();
		forge_files.dedup();

		Ok(forge_files)
	}

	fn find_forge_files_with_gitignore(
		&self,
		search_path: &Path,
		config: &crate::forge_root_config::DiscoveryConfig,
	) -> Result<Vec<PathBuf>, ForgeError> {
		let mut builder = WalkBuilder::new(search_path);

		builder
			.git_ignore(config.use_gitignore)
			.git_exclude(config.use_gitignore)
			.git_global(config.use_gitignore);

		if let Some(max_depth) = config.max_depth {
			builder.max_depth(Some(max_depth));
		}

		for exclude_pattern in &config.exclude {
			builder.add_custom_ignore_filename(exclude_pattern);
		}

		let mut forge_files = Vec::new();

		for result in builder.build() {
			let entry = result.map_err(|e| ForgeError::Other(e.into()))?;

			if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) && entry.file_name().to_str() == Some("FORGE") {
				if self.is_path_excluded(entry.path(), config) {
					continue;
				}

				forge_files.push(entry.path().to_path_buf());
			}
		}

		Ok(forge_files)
	}

	fn find_forge_files_simple(
		&self,
		search_path: &Path,
		config: &crate::forge_root_config::DiscoveryConfig,
	) -> Result<Vec<PathBuf>, ForgeError> {
		let mut builder = WalkDir::new(search_path);

		if let Some(max_depth) = config.max_depth {
			builder = builder.max_depth(max_depth);
		}

		let forge_files: Vec<PathBuf> = builder
			.into_iter()
			.filter_map(|e| e.ok())
			.filter(|e| e.file_name().to_str() == Some("FORGE") && !self.is_path_excluded(e.path(), config))
			.map(|e| e.path().to_path_buf())
			.collect();

		Ok(forge_files)
	}

	fn is_path_excluded(&self, path: &Path, config: &crate::forge_root_config::DiscoveryConfig) -> bool {
		let path_str = path.to_string_lossy();

		if path_str.contains(&self.forge_root_config.build.cache_dir) {
			return true;
		}

		for exclude_pattern in &config.exclude {
			if path_str.contains(exclude_pattern) {
				return true;
			}
		}

		false
	}

	fn needs_rebuild<'a>(&'a self, rule: &'a Rule) -> Result<(bool, Option<String>), ForgeError> {
		if self.check_dependency_changes(rule)? {
			log::debug!("Rebuilding '{}': dependencies have changed.", rule.name);
			let new_hash = self.calculate_rule_hash(rule)?;
			return Ok((true, Some(new_hash)));
		}

		let processed_inputs = self.process_rule_inputs(rule)?;

		for input_cow in &processed_inputs {
			let input = input_cow.as_ref();
			let input_path = self.path.join(input);
			if input_path.exists() {
				let metadata = std::fs::metadata(&input_path)?;
				let modified = metadata.modified()?;

				if let Some(last_modified) = self.cache.mtimes.get(input) {
					if modified > *last_modified.value() {
						log::debug!("Rebuilding '{}': input '{}' was modified.", rule.name, input);
						self.explain_trace.entry(rule.name.clone()).or_default().push(format!("Input file modified: {}", input));
						self.cache.file_hashes.remove(input);
						let new_hash = self.calculate_rule_hash(rule)?;
						return Ok((true, Some(new_hash)));
					}
				} else {
					log::debug!("Rebuilding '{}': input '{}' not found in mtime cache.", rule.name, input);
					self.explain_trace.entry(rule.name.clone()).or_default().push(format!("Input file missing in cache: {}", input));
					let new_hash = self.calculate_rule_hash(rule)?;
					return Ok((true, Some(new_hash)));
				}
			} else {
				// Dependency rule changed or missing file
				self.explain_trace.entry(rule.name.clone()).or_default().push(format!("Input dependency changed or file missing: {}", input));
			}
		}

		for output in &rule.outputs {
			let output_path = self.path.join(output);
			if !output_path.exists() {
				log::debug!("Rebuilding '{}': output '{}' is missing.", rule.name, output);
				self.explain_trace.entry(rule.name.clone()).or_default().push(format!("Output file missing: {}", output));
				let new_hash = self.calculate_rule_hash(rule)?;
				return Ok((true, Some(new_hash)));
			}
		}

		let new_hash = self.calculate_rule_hash(rule)?;
		if let Some(old_hash) = self.cache.rule_hashes.get(&rule.name)
			&& *old_hash.value() == new_hash
		{
			log::info!("Skipping rule '{}' (up-to-date)", rule.name);
			return Ok((false, None));
		}

		self.explain_trace.entry(rule.name.clone()).or_default().push("Command, environment, or toolchain hash changed".to_string());
		Ok((true, Some(new_hash)))
	}

	fn check_dependency_changes<'a>(&'a self, rule: &'a Rule) -> Result<bool, ForgeError> {
		for input in &rule.inputs {
			if let Some(dep_rule_name) = self.output_map.get(input)
				&& let Some(artifact_metadata) = self.cache.artifact_metadata.get(dep_rule_name.value())
				&& let Some(rule_metadata) = self.cache.artifact_metadata.get(&rule.name)
				&& artifact_metadata.created > rule_metadata.created
			{
				return Ok(true);
			}
		}
		Ok(false)
	}

	fn calculate_rule_hash<'a>(&'a self, rule: &'a Rule) -> Result<String, ForgeError> {
		let mut hasher = Hasher::new();

		// Include build profile fingerprint in cache key
		hasher.update(self.build_profile.fingerprint().as_bytes());

		// Include hermetic policy fingerprint in cache key
		hasher.update(format!("{:?}", self.hermetic_policy.mode).as_bytes());
		if self.hermetic_policy.trace_access {
			hasher.update(b"trace_access");
		}
		if self.hermetic_policy.why_non_hermetic {
			hasher.update(b"why_non_hermetic");
		}

		// Include toolchain fingerprints in cache key
		let toolchain_fingerprint = self.calculate_toolchain_fingerprint()?;
		hasher.update(toolchain_fingerprint.as_bytes());

		hasher.update(rule.command.as_bytes());
		for arg in &rule.args {
			hasher.update(arg.as_bytes());
		}
		for (key, val) in &rule.env {
			hasher.update(key.as_bytes());
			hasher.update(val.as_bytes());
		}

		let input_hashes: Result<Vec<String>, ForgeError> = rule
			.inputs
			.par_iter()
			.map(|input| {
				let input_path = self.path.join(input);
				if input_path.exists() {
					let metadata = std::fs::metadata(&input_path)?;
					let modified = metadata.modified()?;

					if let Some(cached_hash) = self.cache.file_hashes.get(input)
						&& let Some(last_modified) = self.cache.mtimes.get(input)
						&& modified <= *last_modified.value()
					{
						return Ok(cached_hash.value().clone());
					}

					let mut file_hasher = Hasher::new();
					file_hasher.update(&metadata.len().to_le_bytes());
					file_hasher.update(&modified.duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_le_bytes());

					if metadata.len() < 1024 * 1024 {
						let content = std::fs::read(&input_path)?;
						file_hasher.update(&content);
					}

					let hash = file_hasher.finalize().to_hex().to_string();
					self.cache.file_hashes.insert(input.to_string(), hash.clone());
					self.cache.mtimes.insert(input.to_string(), modified);
					Ok(hash)
				} else if let Some(dep_rule_name) = self.output_map.get(input) {
					if let Some(dep_hash) = self.cache.rule_hashes.get(dep_rule_name.value()) {
						Ok(dep_hash.value().clone())
					} else {
						Ok("".to_string())
					}
				} else {
					Ok("".to_string())
				}
			})
			.collect();

		for hash in input_hashes? {
			hasher.update(hash.as_bytes());
		}
		Ok(hasher.finalize().to_hex().to_string())
	}

	fn expand_args<'a>(&'a self, args: &'a [String]) -> Result<Vec<Cow<'a, str>>, ForgeError> {
		let mut final_args = Vec::new();
		for arg in args {
			if let Some(path_str) = arg.strip_prefix('@') {
				let file_path = self.path.join(path_str);
				let content = std::fs::read_to_string(&file_path)
					.with_context(|| format!("Failed to read dynamic args file: {}", file_path.display()))?;

				for line in content.lines() {
					if let Some(flag) = line.strip_prefix("cargo:rustc-link-lib=") {
						final_args.push(Cow::Borrowed("-l"));
						final_args.push(Cow::Owned(flag.to_string()));
					} else if let Some(path) = line.strip_prefix("cargo:rustc-link-search=") {
						final_args.push(Cow::Borrowed("-L"));
						final_args.push(Cow::Owned(path.to_string()));
					} else if let Some(cfg) = line.strip_prefix("cargo:rustc-cfg=") {
						final_args.push(Cow::Owned(format!("--cfg={}", cfg)));
					}
				}
			} else {
				final_args.push(Cow::Borrowed(arg.as_str()));
			}
		}
		Ok(final_args)
	}

	pub fn execute_build_graph(&self) -> Result<(), ForgeError> {
		let batches = self.create_parallel_batches()?;
		let total_rules: usize = batches.iter().map(|batch| batch.len()).sum();
		let mut completed_rules = 0;
		let start_time = Instant::now();

		let multi = MultiProgress::new();
		let style = ProgressStyle::default_bar()
			.template("{msg:.green} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {eta}")
			.unwrap()
			.progress_chars("=> ");

		let global_pb = multi.add(ProgressBar::new(total_rules as u64));
		global_pb.set_style(style.clone());
		global_pb.set_message("Building...");

		let total_batches = batches.len();
		for (i, batch) in batches.iter().enumerate() {
			let batch_start = Instant::now();

			let batch_pb = multi.add(ProgressBar::new(batch.len() as u64));
			batch_pb.set_style(style.clone());
			batch_pb.set_message(format!("Batch {}/{}: ", i + 1, total_batches));

			log::info!("\nExecuting batch {}/{}: {:?}", i + 1, total_batches, batch);

			let batch_rules: Vec<String> = batch.iter().cloned().collect();
			let results: Vec<Result<(), ForgeError>> = batch_rules
				.par_iter()
				.enumerate()
				.map(|(idx, rule_name)| {
					let result = self.execute_rule(rule_name);
					batch_pb.inc(1);
					global_pb.inc(1);
					result
				})
				.collect();

			for result in results {
				result?;
			}

			batch_pb.finish_with_message(format!("Batch {}/{} complete", i + 1, total_batches));
			multi.remove(&batch_pb);

			completed_rules += batch.len();
			let elapsed = start_time.elapsed();
			let batch_elapsed = batch_start.elapsed();
			let progress = (completed_rules as f64 / total_rules as f64) * 100.0;
			let estimated_total = if completed_rules > 0 {
				elapsed.as_secs_f64() * (total_rules as f64 / completed_rules as f64)
			} else {
				0.0
			};
			let remaining = estimated_total - elapsed.as_secs_f64();

			global_pb.set_message(format!(
				"Building... {}/{} rules ({:.1}%) ETA: {:.1}s",
				completed_rules,
				total_rules,
				progress,
				remaining.max(0.0)
			));

			log::info!(
				"Batch completed in {:.2}s. Progress: {}/{} rules ({:.1}%). ETA: {:.1}s",
				batch_elapsed.as_secs_f64(),
				completed_rules,
				total_rules,
				progress,
				remaining.max(0.0)
			);
		}

		global_pb.finish_with_message(format!("Build complete! {} rules", total_rules));
		multi.remove(&global_pb);

		let total_elapsed = start_time.elapsed();
		log::info!(
			"\nBuild completed in {:.2}s. Processed {} rules across {} batches.",
			total_elapsed.as_secs_f64(),
			total_rules,
			batches.len()
		);

		Ok(())
	}

	fn execute_rule<'a>(&'a self, rule_name: &'a str) -> Result<(), ForgeError> {
		let rule_ref = self.build_graph.get(rule_name).unwrap();
		let (should_build, new_hash_opt) = self.needs_rebuild(rule_ref.value())?;

		if !should_build {
			return Ok(());
		}
		let new_hash = new_hash_opt.ok_or_else(|| {
			ForgeError::Other(anyhow::anyhow!(
				"Internal error: Rule '{}' needed rebuild but no new hash was calculated.",
				rule_name
			))
		})?;

		let artifact_path = self.cas_path.join(&new_hash);

		if artifact_path.exists() {
			log::info!("Restoring rule '{}' outputs from cache", rule_name);

			let is_compressed = if let Some(metadata) = self.cache.artifact_metadata.get(rule_name) {
				metadata.compressed
			} else {
				false
			};

			for output_rel_path in &rule_ref.value().outputs {
				let output_filename = Path::new(output_rel_path)
					.file_name()
					.ok_or_else(|| ForgeError::Other(anyhow::anyhow!("Invalid output path: {}", output_rel_path)))?
					.to_string_lossy()
					.to_string();

				let dest_path = self.path.join(output_rel_path);
				if let Some(parent) = dest_path.parent() {
					std::fs::create_dir_all(parent)?;
				}

				let compressed_path = artifact_path.join(&output_filename).with_extension("lz4");
				let src_path = artifact_path.join(&output_filename);

				if compressed_path.exists() {
					self.decompress_file(&compressed_path, &dest_path)?;
				} else if src_path.is_dir() {
					self.copy_dir_all(&src_path, &dest_path)?;
				} else if src_path.exists() {
					std::fs::copy(&src_path, &dest_path).with_context(|| {
						format!(
							"Failed to copy cached artifact from {} to {}",
							src_path.display(),
							dest_path.display()
						)
					})?;
				} else {
					return Err(ForgeError::Other(anyhow::anyhow!(
						"Artifact not found in cache for rule '{}': {}",
						rule_name,
						output_filename
					)));
				}
			}
			self.cache.rule_hashes.insert(rule_name.to_string(), new_hash);
			return Ok(());
		}

		log::info!("Running rule: '{}'", rule_name);

		for output in &rule_ref.value().outputs {
			if let Some(parent) = Path::new(output).parent() {
				std::fs::create_dir_all(self.path.join(parent))?;
			}
		}

		// Create ActionSpec for hermetic execution
		let final_args = self.expand_args(&rule_ref.value().args)?;
		let args_strings: Vec<String> = final_args.iter().map(|cow| cow.to_string()).collect();

		let action_spec = ActionSpec::new(rule_name)
			.with_command(&rule_ref.value().command)
			.with_args(args_strings)
			.with_inputs(rule_ref.value().inputs.iter().map(|p| crate::hermetic::ActionInput {
				src: self.path.join(p),
				dest: PathBuf::from(p),
			}).collect())
			.with_outputs(rule_ref.value().outputs.iter().map(|p| self.path.join(p)).collect())
			.with_env(rule_ref.value().env.clone())
			.with_workdir(rule_ref.value().workdir.clone());

		// Execute using SandboxRunner if not in off mode
		let output = if self.hermetic_policy.mode == crate::hermetic::PolicyMode::Off {
			// Direct execution for off mode
			let mut cmd = std::process::Command::new(&rule_ref.value().command);
			let args_refs: Vec<&str> = final_args.iter().map(|cow| cow.as_ref()).collect();
			cmd.args(&args_refs)
				.envs(&rule_ref.value().env)
				.current_dir(&rule_ref.value().workdir);

			log::debug!(
				"Executing command (non-hermetic): {:?} {:?} (workdir: {:?})",
				cmd.get_program(),
				cmd.get_args().collect::<Vec<_>>(),
				rule_ref.value().workdir
			);

			match cmd.output() {
				Ok(o) => o,
				Err(e) => {
					if e.kind() == std::io::ErrorKind::NotFound {
						return Err(ForgeError::BuildFailed {
							rule: rule_name.to_string(),
							error: format!("Command not found: '{}'. Is it installed?", rule_ref.value().command),
						});
					}
					return Err(e).context(format!("Failed to execute command: {}", rule_ref.value().command))?;
				}
			}
		} else {
			// Use SandboxRunner with toolchain paths
			let toolchain_paths = self.get_toolchain_paths();
			let sandbox_dir = self.cas_path.join("sandbox").join(rule_name);
			let runner = SandboxRunner::new(self.hermetic_policy.clone(), sandbox_dir).with_toolchain_paths(toolchain_paths);

			log::debug!(
				"Executing command (hermetic): {:?} {:?} (workdir: {:?})",
				rule_ref.value().command,
				final_args.iter().map(|c| c.as_ref()).collect::<Vec<_>>(),
				rule_ref.value().workdir
			);

			match runner.execute(&action_spec) {
				Ok(result) => {
					let output = std::process::Output {
						status: if result.success {
							std::process::ExitStatus::default()
						} else {
							std::process::ExitStatus::from_raw(1)
						},
						stdout: result.stdout.as_bytes().to_vec(),
						stderr: result.stderr.as_bytes().to_vec(),
					};
					output
				}
				Err(e) => {
					return Err(ForgeError::BuildFailed {
						rule: rule_name.to_string(),
						error: format!("Hermetic execution failed: {}", e),
					});
				}
			}
		};

		if !output.status.success() {
			let stderr = String::from_utf8_lossy(&output.stderr);
			let stdout = String::from_utf8_lossy(&output.stdout);
			return Err(ForgeError::BuildFailed {
				rule: rule_name.to_string(),
				error: format!("STDOUT:\n{}\n\nSTDERR:\n{}", stdout, stderr),
			});
		}

		std::fs::create_dir_all(&artifact_path)?;
		let mut artifact_metadata = crate::cache::ArtifactMetadata {
			size: 0,
			created: std::time::SystemTime::now(),
			compressed: false,
			dependencies: rule_ref.value().inputs.clone(),
		};

		for output_rel_path in &rule_ref.value().outputs {
			let src_path = self.path.join(output_rel_path);

			let output_filename = Path::new(output_rel_path)
				.file_name()
				.ok_or_else(|| ForgeError::Other(anyhow::anyhow!("Invalid output path: {}", output_rel_path)))?
				.to_string_lossy()
				.to_string();
			let dest_path = artifact_path.join(&output_filename);

			let src_metadata = std::fs::metadata(&src_path).with_context(|| format!("Output path not found: {}", src_path.display()))?;
			if src_metadata.is_dir() {
				self.copy_dir_all(&src_path, &dest_path)?;
			} else {
				artifact_metadata.size += src_metadata.len();
				if src_metadata.len() > 1024 * 1024 {
					let compressed_path = dest_path.with_extension("lz4");
					self.compress_file(&src_path, &compressed_path)?;
					artifact_metadata.compressed = true;
				} else {
					std::fs::copy(&src_path, &dest_path).with_context(|| {
						format!(
							"Failed to copy artifact from {} to cache at {}",
							src_path.display(),
							dest_path.display()
						)
					})?;
				}
			}
		}

		self.cache.artifact_metadata.insert(rule_name.to_string(), artifact_metadata);

		self.cache.rule_hashes.insert(rule_name.to_string(), new_hash);
		for input in &rule_ref.value().inputs {
			let input_path = self.path.join(input);
			if input_path.exists() {
				let modified = std::fs::metadata(input_path)?.modified()?;
				self.cache.mtimes.insert(input.to_string(), modified);
			}
		}

		Ok(())
	}

	fn create_parallel_batches(&self) -> Result<Vec<Vec<String>>, ForgeError> {
		let all_rules: Vec<String> = self.build_graph.iter().map(|r| r.key().to_string()).collect();
		let mut reverse_deps: HashMap<String, Vec<String>> = HashMap::new();
		let mut in_degrees: HashMap<String, usize> = self.build_graph.iter().map(|r| (r.key().to_string(), 0)).collect();
		let mut rule_complexity: HashMap<String, f64> = HashMap::new();

		for rule_ref in self.build_graph.iter() {
			let name = rule_ref.key();
			let rule = rule_ref.value();
			let complexity = self.calculate_rule_complexity(rule);
			rule_complexity.insert(name.to_string(), complexity);
		}

		for rule_ref in self.build_graph.iter() {
			let name = rule_ref.key();
			let rule = rule_ref.value();
			println!("@@@ DEBUG: Rule '{}' has deps: {:?}", name, rule.dependencies);

			for input in &rule.inputs {
				if let Some(dep_rule_name) = self.output_map.get(input) {
					reverse_deps
						.entry(dep_rule_name.value().to_string())
						.or_default()
						.push(name.to_string());
					if let Some(degree) = in_degrees.get_mut(name) {
						*degree += 1;
					}
				}
			}

			for dep_rule_name in &rule.dependencies {
				if self.build_graph.contains_key(dep_rule_name) {
					reverse_deps
						.entry(dep_rule_name.to_string())
						.or_default()
						.push(name.to_string());
					if let Some(degree) = in_degrees.get_mut(name) {
						*degree += 1;
					}
				}
			}
		}

		let mut queue: Vec<String> = in_degrees
			.iter()
			.filter(|&(_, &degree)| degree == 0)
			.map(|(name, _)| name.to_string())
			.collect();
		let mut batches = Vec::new();
		let mut processed_count = 0;

		while !queue.is_empty() {
			queue.sort_by(|a, b| {
				let complexity_a = rule_complexity.get(a).unwrap_or(&1.0);
				let complexity_b = rule_complexity.get(b).unwrap_or(&1.0);
				complexity_a.partial_cmp(complexity_b).unwrap_or(std::cmp::Ordering::Equal)
			});

			let current_batch = self.create_balanced_batch(&queue, &rule_complexity);
			processed_count += current_batch.len();
			queue.retain(|rule| !current_batch.contains(rule));

			for rule_name in &current_batch {
				if let Some(dependents) = reverse_deps.get(rule_name) {
					for dependent in dependents {
						if let Some(degree) = in_degrees.get_mut(dependent) {
							*degree -= 1;
							if *degree == 0 {
								queue.push(dependent.to_string());
							}
						}
					}
				}
			}
			batches.push(current_batch);
		}

		if processed_count < self.build_graph.len() {
			let cycle_nodes: Vec<_> = in_degrees
				.iter()
				.filter(|&(_, &d)| d > 0)
				.map(|(n, _)| n.to_string())
				.collect();
			let suggestions = self.generate_cycle_suggestions(&cycle_nodes);
			return Err(ForgeError::CircularDependency {
				cycle: cycle_nodes.join(" → "),
				suggestions,
			});
		}

		self.check_dependency_conflicts()?;

		Ok(batches)
	}

	fn calculate_rule_complexity<'a>(&'a self, rule: &'a Rule) -> f64 {
		let mut complexity = 1.0;

		complexity += (rule.inputs.len() + rule.outputs.len()) as f64 * 0.1;

		if rule.command.contains("rustc") || rule.command.contains("gcc") || rule.command.contains("clang") {
			complexity += 5.0;
		} else if rule.command.contains("cargo") {
			complexity += 3.0;
		} else {
			complexity += 1.0;
		}

		complexity += rule.env.len() as f64 * 0.05;

		complexity
	}

	fn create_balanced_batch<'a>(
		&'a self,
		available_rules: &'a [String],
		complexity: &'a HashMap<String, f64>,
	) -> Vec<String> {
		let max_batch_size = num_cpus::get().min(available_rules.len());
		let mut batch = Vec::new();
		let mut total_complexity = 0.0;
		let target_complexity = 10.0;

		for rule in available_rules {
			let rule_complexity = complexity.get(rule).unwrap_or(&1.0);

			if (batch.len() < max_batch_size && total_complexity + rule_complexity <= target_complexity) || batch.is_empty()
			{
				batch.push(rule.to_string());
				total_complexity += rule_complexity;
			} else {
				break;
			}
		}

		batch
	}

	fn generate_cycle_suggestions<'a>(&'a self, cycle_nodes: &'a [String]) -> String {
		if cycle_nodes.len() <= 2 {
			return format!(
				"Consider removing the dependency between '{}' and '{}'",
				cycle_nodes[0], cycle_nodes[1]
			);
		}

		let mut max_deps = 0;
		let mut suggested_rule = &cycle_nodes[0];

		for rule_name in cycle_nodes {
			if let Some(rule) = self.build_graph.get(rule_name)
				&& rule.inputs.len() > max_deps
			{
				max_deps = rule.inputs.len();
				suggested_rule = rule_name;
			}
		}

		format!(
			"Consider removing one of the dependencies from rule '{}' (has {} dependencies)",
			suggested_rule, max_deps
		)
	}

	fn check_dependency_conflicts(&self) -> Result<(), ForgeError> {
		let mut output_to_rules: HashMap<String, Vec<String>> = HashMap::new();

		for rule_ref in self.build_graph.iter() {
			let rule_name = rule_ref.key();
			let rule = rule_ref.value();
			for output in &rule.outputs {
				output_to_rules
					.entry(output.to_string())
					.or_default()
					.push(rule_name.to_string());
			}
		}

		for (output, rules) in output_to_rules {
			if rules.len() > 1 {
				return Err(ForgeError::DependencyConflict {
					conflict: format!("Multiple rules produce the same output '{}': {}", output, rules.join(", ")),
					suggestion: "Ensure each output file is produced by only one rule, or rename conflicting outputs"
						.to_string(),
				});
			}
		}

		Ok(())
	}

	fn compress_file<'a>(&'a self, src: &'a Path, dest: &'a Path) -> Result<(), ForgeError> {
		use lz4::EncoderBuilder;
		use std::io::Write;

		let content = std::fs::read(src)?;
		let mut encoder = EncoderBuilder::new().level(1).build(std::fs::File::create(dest)?)?;
		encoder.write_all(&content)?;
		let (_output, result) = encoder.finish();
		result?;
		Ok(())
	}

	fn decompress_file<'a>(&'a self, src: &'a Path, dest: &'a Path) -> Result<(), ForgeError> {
		use lz4::Decoder;
		use std::io::Read;

		let file = std::fs::File::open(src)?;
		let mut decoder = Decoder::new(file)?;
		let mut contents = Vec::new();
		decoder.read_to_end(&mut contents)?;
		std::fs::write(dest, contents)?;
		Ok(())
	}

	fn copy_dir_all(&self, src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<(), ForgeError> {
		std::fs::create_dir_all(&dst)?;
		for entry in std::fs::read_dir(src)? {
			let entry = entry?;
			let ty = entry.file_type()?;
			if ty.is_dir() {
				self.copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
			} else {
				std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
			}
		}
		Ok(())
	}
}
