use crate::{cache::BuildCache, config::Config, error::ForgeError, forge_root_config::ForgeRootConfig, lua_api};
use anyhow::Context;
use blake3::Hasher;
use dashmap::DashMap;
use ignore::WalkBuilder;
use mlua::{Lua, UserData};
use rayon::prelude::*;
use std::{
	borrow::Cow,
	collections::HashMap,
	path::{Path, PathBuf},
	sync::Arc,
	time::Instant,
};
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct Rule {
	pub name: String,
	pub command: String,
	pub args: Vec<String>,
	pub env: HashMap<String, String>,
	pub inputs: Vec<String>,
	pub outputs: Vec<String>,
	pub dependencies: Vec<String>,
	pub workdir: PathBuf,
}

impl UserData for Rule {}

#[derive(Clone)]
pub struct Project {
	pub path: PathBuf,
	pub config: Config,
	pub forge_root_config: ForgeRootConfig,
	pub build_graph: Arc<DashMap<String, Rule>>,
	pub output_map: Arc<DashMap<String, String>>,
	pub cache: BuildCache,
	cas_path: PathBuf,
	lua: Lua,
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

	pub fn new(path: PathBuf, config: Config) -> Result<Self, ForgeError> {
		let forge_root_path = path.join("FORGE_ROOT");
		let forge_root_config = ForgeRootConfig::load(&forge_root_path).map_err(|_| ForgeError::ForgeRootNotFound {
			path: forge_root_path.display().to_string(),
		})?;

		let output_dir = path.join(&forge_root_config.build.cache_dir);
		let cas_path = output_dir.join("cas");
		std::fs::create_dir_all(&output_dir)?;
		std::fs::create_dir_all(&cas_path)?;

		if !path.join("prelude").exists() {
			return Err(ForgeError::PreludeNotFound(path.join("prelude").display().to_string()));
		}

		let cache_path = output_dir.join("cache.json");
		let cache = BuildCache::load(&cache_path);

		cache.validate_and_clean(&path);

		Ok(Self {
			path,
			config,
			forge_root_config,
			build_graph: Arc::new(DashMap::new()),
			output_map: Arc::new(DashMap::new()),
			cache,
			cas_path,
			lua: Lua::new(),
		})
	}

	fn setup_lua_environment(&self) -> Result<(), ForgeError> {
		lua_api::init::setup_lua_environment(&self.lua, self)?;
		Ok(())
	}

	pub fn run(&mut self) -> Result<(), ForgeError> {
		self.setup_lua_environment()?;

		let forge_files = self.find_forge_files(&self.path)?;

		for forge_file in &forge_files {
			log::debug!("Loading FORGE file: {}", forge_file.display());
			let content = std::fs::read_to_string(forge_file)?;

			if content.trim().is_empty() {
				return Err(ForgeError::InvalidForgeFile {
					file: forge_file.display().to_string(),
					error: "FORGE file is empty".to_string(),
					suggestion: "Add build rules to your FORGE file".to_string(),
				});
			}

			if !content.contains("rule") && !content.contains("require") {
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

		self.execute_build_graph()?;

		let cache_path = self.path.join("forge-out").join("cache.json");
		self.cache.save(&cache_path).context("Failed to save build cache")?;

		Ok(())
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
						self.cache.file_hashes.remove(input);
						let new_hash = self.calculate_rule_hash(rule)?;
						return Ok((true, Some(new_hash)));
					}
				} else {
					log::debug!("Rebuilding '{}': input '{}' not found in mtime cache.", rule.name, input);
					let new_hash = self.calculate_rule_hash(rule)?;
					return Ok((true, Some(new_hash)));
				}
			}
		}

		for output in &rule.outputs {
			let output_path = self.path.join(output);
			if !output_path.exists() {
				log::debug!("Rebuilding '{}': output '{}' is missing.", rule.name, output);
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

	fn execute_build_graph(&self) -> Result<(), ForgeError> {
		let batches = self.create_parallel_batches()?;
		let total_rules: usize = batches.iter().map(|batch| batch.len()).sum();
		let mut completed_rules = 0;
		let start_time = Instant::now();

		for (i, batch) in batches.iter().enumerate() {
			let batch_start = Instant::now();
			log::info!("\nExecuting batch {}/{}: {:?}", i + 1, batches.len(), batch);

			let results: Vec<Result<(), ForgeError>> =
				batch.par_iter().map(|rule_name| self.execute_rule(rule_name)).collect();

			for result in results {
				result?;
			}

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

			log::info!(
				"Batch completed in {:.2}s. Progress: {}/{} rules ({:.1}%). ETA: {:.1}s",
				batch_elapsed.as_secs_f64(),
				completed_rules,
				total_rules,
				progress,
				remaining.max(0.0)
			);
		}

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

				if is_compressed {
					let compressed_path = artifact_path.join(&output_filename).with_extension("lz4");
					if compressed_path.exists() {
						self.decompress_file(&compressed_path, &dest_path)?;
					}
				} else {
					let src_path = artifact_path.join(&output_filename);
					std::fs::copy(&src_path, &dest_path).with_context(|| {
						format!(
							"Failed to copy cached artifact from {} to {}",
							src_path.display(),
							dest_path.display()
						)
					})?;
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

		let final_args = self.expand_args(&rule_ref.value().args)?;
		let mut cmd = std::process::Command::new(&rule_ref.value().command);

		let args_refs: Vec<&str> = final_args.iter().map(|cow| cow.as_ref()).collect();
		cmd.args(&args_refs)
			.envs(&rule_ref.value().env)
			.current_dir(&rule_ref.value().workdir);

		log::debug!(
			"Executing command: {:?} {:?} (workdir: {:?})",
			cmd.get_program(),
			cmd.get_args().collect::<Vec<_>>(),
			rule_ref.value().workdir
		);

		let output = cmd.output()?;

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

			let src_metadata = std::fs::metadata(&src_path)?;
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
}
