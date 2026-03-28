use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::cache::{CacheDb, CacheGC, GCResult};
use crate::hermetic::{HermeticPolicy, PolicyMode};

lazy_static::lazy_static! {
	static ref last_test_stdout: Mutex<String> = Mutex::new(String::new());
	static ref last_test_stderr: Mutex<String> = Mutex::new(String::new());
}

#[derive(Subcommand, Debug)]
enum CacheCommand {
	Stats,
	List {
		#[arg(short, long, default_value = "20")]
		limit: usize,
	},
	Prune {
		#[arg(long, help = "Maximum cache size (e.g., 10GB)")]
		max_size: Option<String>,
		#[arg(long, help = "Remove artifacts older than N days")]
		older_than: Option<u64>,
	},
	Clean,
}

#[derive(Subcommand, Debug)]
enum DepsCommand {
	List,
	Sync,
}

#[derive(Subcommand, Debug)]
enum ToolchainCommand {
	Sync,
	List,
	Verify,
}

mod cache;
mod config;
mod coverage;
mod error;
mod forge_root_config;
mod graph;
mod hermetic;
mod lua_api;
mod package;
mod platform;
mod profile;
mod project;
mod query;
use crate::query::QueryEngine;
mod source;
mod target;
mod watch;
mod worker;
mod workspace;

use std::process::Command;

#[derive(Parser, Debug)]
#[command(version, about, long_about = "A universal, concurrent build system powered by Lua")]
struct Cli {
	#[arg(short, long, value_name = "PATH", default_value = ".")]
	project: PathBuf,

	#[command(subcommand)]
	command: Option<Commands>,

	#[arg(
		short,
		long,
		help = "Build specific target(s) (can be used multiple times). Required when no subcommand is provided."
	)]
	target: Vec<String>,

	#[command(flatten)]
	verbose: clap_verbosity_flag::Verbosity,

	#[arg(long, global = true, help = "Hermetic policy mode: off, warn, or strict")]
	hermetic: Option<String>,

	#[arg(long, help = "Trace file access during execution and report undeclared reads/writes")]
	trace_access: bool,

	#[arg(long, help = "Explain why an action is non-hermetic (shows env, inputs, toolchain issues)")]
	why_non_hermetic: bool,

	#[arg(long, global = true, help = "Build profile to use (e.g. debug, release, asan)")]
	profile: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
	Build {
		#[arg(short, long, help = "Build specific target(s) (can be used multiple times)")]
		target: Vec<String>,

		#[arg(short, long, help = "Build specific component(s) (can be used multiple times)")]
		component: Vec<String>,
	},

	Run {
		#[arg(short, long)]
		target: Option<String>,

		#[arg(short, long, help = "Run specific component")]
		component: Option<String>,
	},

	Test {
		#[arg(short, long, help = "Target to be used for testing (required)")]
		target: String,

		#[arg(short, long, help = "Test specific component")]
		component: Option<String>,

		#[arg(short, long, help = "Filter tests by pattern")]
		filter: Option<String>,

		#[arg(long, help = "Run impacted tests based on file changes")]
		changed: Option<Vec<String>>,

		#[arg(long, help = "Explain why a test is not using cached result")]
		explain_cache: bool,

		#[arg(long, help = "Disable test result cache")]
		nocache: bool,

		#[arg(long, help = "Cache test results: auto|yes|no", default_value = "auto")]
		cache_test_results: String,

		#[arg(long, help = "Generate a flake report across historical test runs")]
		flake_report: bool,

		#[arg(long, help = "Only re-run tests that failed previously")]
		rerun_failed: bool,

		#[arg(long, help = "Output format (e.g., junit:path/to/results.xml)")]
		output: Option<String>,
	},

	Coverage {
		#[arg(short, long, help = "Target to be used for coverage compilation")]
		target: Option<String>,

		#[arg(short, long, help = "Filter tests and covered components by pattern")]
		filter: Option<String>,

		#[arg(long, help = "Output format (e.g., lcov:cov.info, html:cov_dir/, json:cov.json)")]
		output: Option<String>,
	},

	Clean {
		#[arg(long, help = "Expunge all downloaded toolchains and entire cache")]
		expunge: bool,

		#[arg(long, help = "Clear only the build artifact cache")]
		cache: bool,

		#[arg(long, help = "Clear only test results")]
		test: bool,
	},

	Init {
		#[arg(long, help = "Project name (defaults to directory name)")]
		name: Option<String>,

		#[arg(long, help = "Force overwrite existing FORGE_ROOT")]
		force: bool,
	},

	Migrate {
		#[arg(long, help = "Force overwrite existing FORGE_ROOT")]
		force: bool,
	},

	Types {
		#[arg(short, long, help = "Output path for types.lua file", default_value = "types.lua")]
		output: PathBuf,
	},

	Cache {
		#[command(subcommand)]
		action: CacheCommand,
	},

	Deps {
		#[command(subcommand)]
		action: DepsCommand,
	},

	Toolchain {
		#[command(subcommand)]
		action: ToolchainCommand,
	},

	Query {
		#[arg(help = "The query expression to evaluate (e.g. 'deps(//lib:all)')")]
		expression: String,

		#[arg(short, long, help = "Output format: text|json|dot|count", default_value = "text")]
		format: String,

		#[arg(short, long, help = "Write output to a file instead of stdout")]
		output: Option<PathBuf>,
	},

	Watch {
		#[arg(long, help = "Only watch and build specific targets (e.g., linux_x64)")]
		target: Vec<String>,

		#[arg(long, help = "Only watch and build specific components (e.g., //lib:math)")]
		component: Vec<String>,

		#[arg(long, help = "Run tests after build")]
		test: bool,
	},

	Explain {
		#[arg(help = "The target or component to explain staleness for")]
		target: String,
	},


	Fmt {
		#[arg(long, help = "Check formatting without modifying files")]
		check: bool,
	},

	CompileCommands {
		#[arg(short, long, help = "Only generate entries for this target (default: all)")]
		target: Vec<String>,

		#[arg(short, long, help = "Output path (default: compile_commands.json in project root)")]
		output: Option<std::path::PathBuf>,

		#[arg(long, default_value = "false", help = "Merge into existing compile_commands.json instead of overwriting")]
		merge: bool,
	},

	Graph {
		#[arg(short, long, help = "Only show nodes related to these targets")]
		target: Vec<String>,

		#[arg(short, long, help = "Only show nodes related to these components")]
		component: Vec<String>,

		#[arg(short, long, help = "Output format: dot|json", default_value = "dot")]
		format: String,
	},
}

fn main() -> Result<()> {
	println!("@@@ FORGE STARTING @@@");
	let cli = Cli::parse();

	// Default to Info level (errors, warnings, info)
	// With -v: Debug level
	// With -vv: Trace level
	let log_level = if cli.verbose.is_present() {
		match cli.verbose.log_level_filter() {
			log::LevelFilter::Error => log::LevelFilter::Debug,
			log::LevelFilter::Warn => log::LevelFilter::Debug,
			log::LevelFilter::Info => log::LevelFilter::Trace,
			l => l,
		}
	} else {
		log::LevelFilter::Info
	};

	env_logger::Builder::new()
		.filter_level(log_level)
		.format_timestamp_secs()
		.init();

	// Parse hermetic policy mode override from CLI
	let cli_mode: Option<crate::hermetic::PolicyMode> = match cli.hermetic.as_deref() {
		Some(mode_str) => match mode_str.parse() {
			Ok(mode) => Some(mode),
			Err(e) => {
				eprintln!("Error: {}", e);
				std::process::exit(1);
			}
		},
		None => None,
	};
    
    // We'll create a dummy policy just to hold the other flags for now, 
    // but Project::new will create the real one.
    // Actually, let's keep the flags.

	log::info!("Hermetic policy override: {:?}", cli_mode);

	let project_path = std::fs::canonicalize(&cli.project)?;

	match cli.command {
		Some(Commands::Build {
			target,
			component,
		}) => {
			if target.is_empty() && component.is_empty() {
				return Err(anyhow::anyhow!(
					"No targets or components specified for build. Use --target and/or --component to specify what to build.\n\
					Example: forge build --target linux_x64_debug\n\
					         forge build --component math_utils\n\
					         forge build --component math_utils --target linux_x64_debug"
				));
			}

			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: target,
				component_filters: component,
				test_mode: false,
				profile: cli.profile.clone(),
			};

			log::info!("Building project at: {}", project_path.display());
			if !config.target_filters.is_empty() {
				log::info!("Target filters: {}", config.target_filters.join(", "));
			}
			if !config.component_filters.is_empty() {
				log::info!("Component filters: {}", config.component_filters.join(", "));
			}

			let mut project = project::Project::new(
				project_path,
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			project.run()?;

			println!("\nBuild completed successfully!");
		}
		Some(Commands::Run { target, component }) => {
			let target_filters = target.iter().cloned().collect();
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters,
				component_filters: if let Some(ref comp) = component {
					vec![comp.clone()]
				} else {
					vec![]
				},
				test_mode: false,
				profile: cli.profile.clone(),
			};

			let mut project = project::Project::new(
				project_path.clone(),
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			project.run()?;

			if let Some(target_name) = target {
				if let Some(comp) = component {
					run_component_target(&project_path, &comp, &target_name)?;
				} else {
					run_target(&project_path, &target_name)?;
				}
			} else {
				run_main_executable(&project_path)?;
			}

			println!("\nBuild and run completed successfully!");
		}
		Some(Commands::Test {
			target,
			component,
			filter,
			changed,
			explain_cache,
			nocache,
			cache_test_results,
			flake_report,
			rerun_failed,
			output,
		}) => {
			let forge_out = project_path.join("forge-out");
			let sqlite_path = forge_out.join("cas").join("cache.db");

			if flake_report {
				log::info!("Generating flake report...");
				if sqlite_path.exists() {
					if let Ok(db) = CacheDb::new(&sqlite_path) {
						if let Ok(flakes) = db.get_flake_report() {
							println!("\n=== FLAKE REPORT ===");
							if flakes.is_empty() {
								println!("Great job! No flaky tests detected continuously.");
							} else {
								for flake in flakes {
									println!("- {} (Target: {}): {} flakes over {} runs", flake.test_name, flake.target, flake.flake_count, flake.run_count);
								}
							}
						}
					}
				} else {
					println!("No test history found.");
				}
				return Ok(());
			}

			let mut component_filters = Vec::new();
			if rerun_failed {
				log::info!("Fetching previously failed tests...");
				if sqlite_path.exists() {
					if let Ok(db) = CacheDb::new(&sqlite_path) {
						if let Ok(failed) = db.get_failed_tests() {
							for f in failed {
								component_filters.push(f.test_name.clone());
							}
						}
					}
				}
				if component_filters.is_empty() {
					println!("No previously failed tests found in cache.");
					return Ok(());
				}
			} else if let Some(ref comp) = component {
				component_filters.push(comp.clone());
			} else if let Some(ref f) = filter {
				component_filters.push(f.clone());
			}

			if let Some(ref files) = changed {
				log::info!("Simulating impacted tests based on changed files: {:?}", files);
				// TODO: Integrate with Reverse Dependencies in BuildGraph to map files to tests.
			}

			if explain_cache {
				log::info!("Cache diagnostics enabled");
			}
			if nocache {
				log::info!("Test result cache disabled");
			}
			match cache_test_results.as_str() {
				"yes" => log::info!("Test result cache: enabled"),
				"no" => log::info!("Test result cache: disabled"),
				_ => log::info!("Test result cache: auto"),
			}

			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: vec![target.clone()],
				component_filters: component_filters.clone(),
				test_mode: true,
				profile: cli.profile.clone(),
			};

			log::info!("Building and testing project at: {}", project_path.display());
			log::info!("Test target: {}", target);
			if let Some(ref comp) = component {
				log::info!("Test component: {}", comp);
			}

			let mut project = project::Project::new(
				project_path.clone(),
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			let mut test_errors = false;
			if let Err(e) = project.run() {
				log::error!("Tests failed: {}", e);
				test_errors = true;
			}

			if let Some(ref out) = output {
				if out.starts_with("junit:") {
					let out_path = out.strip_prefix("junit:").unwrap_or(out);
					log::info!("Writing basic JUnit test report to: {}", out_path);
					let xml = format!(
						"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n  <testsuite name=\"forge_tests\" tests=\"1\" failures=\"{}\">\n    <testcase name=\"{}\"/>\n  </testsuite>\n</testsuites>",
						if test_errors { 1 } else { 0 },
						target
					);
					if let Some(parent) = Path::new(out_path).parent() {
						std::fs::create_dir_all(parent).unwrap_or_default();
					}
					std::fs::write(out_path, xml).unwrap_or_default();
				}
			}

			if test_errors {
				return Err(anyhow::anyhow!("One or more tests failed"));
			}

			println!("\nTest completed successfully!");
		}
		Some(Commands::Coverage {
			target,
			filter,
			output,
		}) => {
			let mut component_filters = Vec::new();
			if let Some(ref f) = filter {
				component_filters.push(f.clone());
			}

			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: target.into_iter().collect(),
				component_filters,
				test_mode: true,
				profile: Some("coverage".to_string()),
			};

			log::info!("Building and calculating coverage at: {}", project_path.display());

			let mut project = project::Project::new(
				project_path.clone(),
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			
			if let Err(e) = project.run_coverage(output.as_deref()) {
				log::error!("Coverage failed: {}", e);
				return Err(anyhow::anyhow!("Coverage failed"));
			}

			println!("\nCoverage completed successfully!");
		}
		Some(Commands::Clean {
			expunge: _,
			cache: _,
			test: _,
		}) => {
			log::info!("Cleaning project at: {}", project_path.display());
			clean_project(&project_path)?;
			println!("\nClean completed successfully!");
		}
		Some(Commands::Init { name, force }) => {
			init_forge_root(&project_path, name, force)?;
		}
		Some(Commands::Migrate { force }) => {
			migrate_to_forge_root(&project_path, force)?;
		}
		Some(Commands::Types { output }) => {
			log::info!("Generating Lua type definitions to: {}", output.display());
			let types_content = lua_api::init::generate_types_lua();
			std::fs::write(&output, types_content)?;
			println!("Generated types.lua at: {}", output.display());
		}
		Some(Commands::Cache { action }) => {
			handle_cache_command(action, &project_path)?;
		}
		Some(Commands::Deps { action }) => {
			handle_deps_command(action)?;
		}
		Some(Commands::Toolchain { action }) => {
			handle_toolchain_command(
				action,
				&project_path,
				cli.verbose.clone(),
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
		}
		Some(Commands::Query { expression, format, output }) => {
			log::info!("Evaluating query: {}", expression);
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: vec![],
				component_filters: vec![],
				test_mode: false,
				profile: cli.profile.clone(),
			};

			let mut project = project::Project::new(
				project_path,
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			project.load_graph()?;

			let graph = project.dependency_graph.lock().unwrap();
			let engine = QueryEngine::new(&graph);
			match engine.evaluate(&expression) {
				Ok(ids) => {
					let output_content = match format.as_str() {
						"json" => {
							let mut components = Vec::new();
							for id in &ids {
								if let Some(comp) = graph.get_component(*id) {
									components.push(comp);
								}
							}
							serde_json::to_string_pretty(&components).unwrap_or_default()
						}
						"dot" => {
							graph.output_dot_subset(&ids, &crate::graph::DotOptions::default_pretty())
						}
						"count" => ids.len().to_string(),
						_ => {
							let mut labels: Vec<String> = Vec::new();
							for id in ids {
								if let Some(comp) = graph.get_component(id) {
									labels.push(format!("{}:{}", comp.target_name, comp.name));
								}
							}
							labels.sort();
							labels.join("\n")
						}
					};

					if let Some(out_path) = output {
						std::fs::write(&out_path, output_content)?;
						println!("Query results written to: {}", out_path.display());
					} else {
						println!("{}", output_content);
					}
				}
				Err(e) => {
					eprintln!("Error: {}", e);
					std::process::exit(1);
				}
			}
		}
		Some(Commands::Watch { target, component, test }) => {
			log::info!("Starting watch mode");
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: target.clone(),
				component_filters: component.clone(),
				test_mode: test,
				profile: cli.profile.clone(),
			};

			if let Err(e) = crate::watch::watch_project(
				project_path,
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			) {
				log::error!("Watch loop crashed: {}", e);
				std::process::exit(1);
			}
		}
		Some(Commands::Explain { target }) => {
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: vec![], // Load everything first for metadata
				component_filters: vec![],
				test_mode: false,
				profile: cli.profile.clone(),
			};

			let mut project = project::Project::new(
				project_path,
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			let _ = project.load_graph(); // We want to explain even if the graph has errors (e.g. visibility violations)

			// 1. Show graph metadata for matched components
			{
				let graph = project.dependency_graph.lock().unwrap();
				let matched_ids = graph.components_matching(&target);
				
				for id in matched_ids {
					if let Some(comp) = graph.get_component(id) {
						println!("\n=== Component: {}:{} ===", comp.target_name, comp.name);
						println!("  Type:      {:?}", comp.component_type);
						println!("  Package:   {}", comp.package.as_str());
						println!("  Visibility: {:?}", comp.visibility);
						if !comp.compatible_with.is_empty() {
							println!("  Platforms: {:?}", comp.compatible_with);
						}

						let deps = graph.dependencies_of(id);
						if !deps.is_empty() {
							println!("  Depends on:");
							for dep_id in deps {
								if let Some(dep) = graph.get_component(dep_id) {
									println!("    - {}:{}", dep.target_name, dep.name);
								}
							}
						}

						let rdeps = graph.reverse_dependencies(id);
						if !rdeps.is_empty() {
							println!("  Needed by:");
							for rdep_id in rdeps {
								if let Some(rdep) = graph.get_component(rdep_id) {
									println!("    - {}:{}", rdep.target_name, rdep.name);
								}
							}
						}
					}
				}
			}

			// 2. Show staleness traces (dry-run)
			log::info!("Dry-running to detect stale components for '{}'...", target);
			project.config.target_filters = vec![target.clone()];
			project.run()?;

			println!("\n=== Cache Stale Traces ===");
			if project.explain_trace.is_empty() {
				println!("✓ All components matched by '{}' are fully cached and up-to-date!", target);
			} else {
				for entry in project.explain_trace.iter() {
					println!("Component '{}':", entry.key());
					for trace in entry.value() {
						println!("  - [Stale] {}", trace);
					}
				}
			}
		}
		Some(Commands::Fmt { check }) => {
			log::info!("Formatting Lua build files with stylua...");

			// Verify stylua is available
			if std::process::Command::new("stylua").arg("--version").output().is_err() {
				eprintln!("Error: 'stylua' is not installed or not available in PATH.");
				eprintln!("Forge uses 'stylua' to format its build and prelude files.");
				eprintln!("Please install it first: cargo install stylua");
				std::process::exit(1);
			}

			let mut files = Vec::new();
			let walker = ignore::WalkBuilder::new(&project_path).build();
			for result in walker {
				if let Ok(entry) = result {
					let path = entry.path();
					if path.is_file() {
						if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
							if name.ends_with(".lua") || name == "FORGE" {
								files.push(path.to_path_buf());
							}
						}
					}
				}
			}

			if files.is_empty() {
				println!("No .lua or FORGE files found to format.");
				return Ok(());
			}

			let mut fmt_failed = false;
			// Process in chunks to avoid OS command line length limits
			for chunk in files.chunks(100) {
				let mut cmd = std::process::Command::new("stylua");
				if check {
					cmd.arg("--check");
				}
				for file in chunk {
					cmd.arg(file);
				}
				match cmd.status() {
					Ok(status) => {
						if !status.success() {
							fmt_failed = true;
						}
					}
					Err(e) => {
						eprintln!("Failed to execute stylua: {}", e);
						std::process::exit(1);
					}
				}
			}

			if fmt_failed {
				eprintln!("Formatting/check failed. See stylua output above.");
				std::process::exit(1);
			} else if check {
				println!("All files formatted correctly!");
			} else {
				println!("Successfully formatted {} files.", files.len());
			}
		}
		Some(Commands::CompileCommands { target, output, merge }) => {
			log::info!("Generating compile_commands.json...");
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: target,
				component_filters: vec![],
				test_mode: false,
				profile: cli.profile.clone(),
			};
			let project = project::Project::new(
				project_path.clone(),
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			project.load_graph()?;

			// Compile-command detectors: match common C/C++ compiler driver names.
			const COMPILER_INDICATORS: &[&str] = &[
				"gcc", "g++", "clang", "clang++", "cc", "c++", "zig cc", "zig c++",
			];
			let is_compile_rule = |cmd: &str| -> bool {
				let base = std::path::Path::new(cmd)
					.file_name()
					.map(|n| n.to_string_lossy().to_lowercase())
					.unwrap_or_default();
				COMPILER_INDICATORS.iter().any(|k| base.contains(k))
			};

			// Collect existing entries if merging
			let out_path = output.unwrap_or_else(|| project_path.join("compile_commands.json"));
			let mut entries: Vec<serde_json::Value> = if merge && out_path.exists() {
				if let Ok(data) = std::fs::read_to_string(&out_path) {
					serde_json::from_str(&data).unwrap_or_default()
				} else {
					vec![]
				}
			} else {
				vec![]
			};

			// Track files already present to avoid duplicates when merging
			let existing_files: std::collections::HashSet<String> = if merge {
				entries.iter()
					.filter_map(|e| e.get("file").and_then(|f| f.as_str()).map(String::from))
					.collect()
			} else {
				std::collections::HashSet::new()
			};

			let mut new_count = 0usize;
			for rule_entry in project.build_graph.iter() {
				let rule = rule_entry.value();
				if !is_compile_rule(&rule.command) {
					continue;
				}

				// Find the primary source file: first .c / .cpp / .cc / .cxx in inputs
				let source_file = rule.inputs.iter().find(|i| {
					matches!(
						std::path::Path::new(i).extension().and_then(|e| e.to_str()),
						Some("c" | "cpp" | "cc" | "cxx" | "C" | "CPP")
					)
				});
				let file_path = match source_file {
					Some(f) => f.clone(),
					None => continue, // skip link/archive rules
				};

				// Make file path absolute if it isn't
				let abs_file = if std::path::Path::new(&file_path).is_absolute() {
					file_path.clone()
				} else {
					rule.workdir.join(&file_path).to_string_lossy().to_string()
				};

				if merge && existing_files.contains(&abs_file) {
					continue;
				}

				// Build the arguments list (for the 'arguments' array form)
				let mut arguments: Vec<String> = vec![rule.command.clone()];
				arguments.extend(rule.args.iter().cloned());

				entries.push(serde_json::json!({
					"directory": rule.workdir.to_string_lossy(),
					"file": abs_file,
					"arguments": arguments,
				}));
				new_count += 1;
			}

			let json = serde_json::to_string_pretty(&entries)?;
			if let Some(parent) = out_path.parent() {
				std::fs::create_dir_all(parent)?;
			}
			std::fs::write(&out_path, json)?;
			println!(
				"compile_commands.json written to {} ({} entries{})",
				out_path.display(),
				new_count,
				if merge { " merged" } else { "" },
			);
		}
		Some(Commands::Graph { target, component, format }) => {
			log::info!("Generating graph of workspace dependencies...");
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: target,
				component_filters: component,
				test_mode: false,
				profile: cli.profile.clone(),
			};
			let project = project::Project::new(
				project_path,
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			project.load_graph()?;
			
			if format == "json" {
				let mut rules = std::collections::HashMap::new();
				for entry in project.build_graph.iter() {
					rules.insert(entry.key().clone(), entry.value().clone());
				}
				println!("{}", serde_json::to_string_pretty(&rules)?);
			} else {
				let mut dot = String::new();
				dot.push_str("digraph BuildGraph {\n");
				dot.push_str("  node [shape=box];\n");
				for entry in project.build_graph.iter() {
					let name = entry.key();
					let rule = entry.value();
					dot.push_str(&format!("  \"{}\" [label=\"{}\"];\n", name, name));
					for dep in &rule.dependencies {
						dot.push_str(&format!("  \"{}\" -> \"{}\";\n", name, dep));
					}
				}
				dot.push_str("}\n");
				println!("{}", dot);
			}
		}
		None => {
			if cli.target.is_empty() {
				return Err(anyhow::anyhow!(
					"No targets specified. Use --target to specify one or more targets to build.\n\
					Example: forge --target linux_x64_debug --target linux_x64_release"
				));
			}

			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: cli.target,
				component_filters: vec![],
				test_mode: false,
				profile: cli.profile.clone(),
			};

			log::info!("Building project at: {}", project_path.display());
			log::info!("Targets: {}", config.target_filters.join(", "));

			let mut project = project::Project::new(
				project_path,
				config,
				cli_mode,
				cli.trace_access,
				cli.why_non_hermetic,
			)?;
			project.run()?;

			println!("\nBuild completed successfully!");
		}
	}

	Ok(())
}

fn init_forge_root(project_path: &Path, name: Option<String>, force: bool) -> Result<()> {
	let forge_root_path = project_path.join("FORGE_ROOT");

	if forge_root_path.exists() && !force {
		return Err(anyhow::anyhow!(
			"FORGE_ROOT already exists at {}. Use --force to overwrite.",
			forge_root_path.display()
		));
	}

	let project_name = name.unwrap_or_else(|| project_path.file_name().unwrap_or_default().to_string_lossy().to_string());

	let config = forge_root_config::ForgeRootConfig::create_default(&project_name);
	config.save(&forge_root_path)?;

	println!("Created FORGE_ROOT configuration at: {}", forge_root_path.display());
	println!("\nNext steps:");
	println!("1. Edit FORGE_ROOT to customize your project configuration");
	println!("2. Create FORGE files in your source directories (src/FORGE, lib/FORGE, etc.)");
	println!("3. Run 'forge build --target <your-target>' to build");

	Ok(())
}

fn migrate_to_forge_root(project_path: &Path, force: bool) -> Result<()> {
	let forge_root_path = project_path.join("FORGE_ROOT");

	if forge_root_path.exists() && !force {
		return Err(anyhow::anyhow!(
			"FORGE_ROOT already exists at {}. Use --force to overwrite.",
			forge_root_path.display()
		));
	}

	let project_name = project_path.file_name().unwrap_or_default().to_string_lossy().to_string();

	let mut config = forge_root_config::ForgeRootConfig::create_default(&project_name);

	let mut suggested_includes = vec![".".to_string()];
	let mut suggested_excludes = vec![];

	for common_dir in &["src", "lib", "examples", "tests", "benches"] {
		if project_path.join(common_dir).exists() {
			suggested_includes.push(common_dir.to_string());
		}
	}

	for common_exclude in &["target", "build", "dist", "node_modules", ".git"] {
		if project_path.join(common_exclude).exists() {
			suggested_excludes.push(common_exclude.to_string());
		}
	}

	config.discovery.include = suggested_includes;
	config.discovery.exclude = suggested_excludes;

	config.save(&forge_root_path)?;

	println!("Created FORGE_ROOT configuration at: {}", forge_root_path.display());
	println!("\nMigration complete! Detected project structure:");
	println!("- Include directories: {}", config.discovery.include.join(", "));
	if !config.discovery.exclude.is_empty() {
		println!("- Exclude directories: {}", config.discovery.exclude.join(", "));
	}
	println!("\nThe FORGE_ROOT file has been created with suggested settings.");
	println!("You can edit it to customize the configuration for your project.");

	Ok(())
}

fn run_target(project_path: &PathBuf, target_name: &str) -> Result<()> {
	let mut target_path = project_path.join(target_name);

	if !target_path.exists() {
		let forge_out = project_path.join("forge-out");
		if forge_out.exists() {
			let target_dir = forge_out.join(target_name);
			if let Some(executable) = find_executable_in_dir(&target_dir, None) {
				target_path = executable;
			}
		}
	}

	if !target_path.exists() {
		return Err(anyhow::anyhow!(
			"Target '{}' not found at {}",
			target_name,
			target_path.display()
		));
	}

	if !target_path.is_file() {
		return Err(anyhow::anyhow!("Target '{}' is not a file", target_name));
	}

	execute_binary(&target_path, project_path)
}

fn run_component_target(project_path: &PathBuf, component_name: &str, target_name: &str) -> Result<()> {
	let forge_out = project_path.join("forge-out");
	if !forge_out.exists() {
		return Err(anyhow::anyhow!("forge-out directory not found at {}", forge_out.display()));
	}

	let target_dir = forge_out.join(target_name);
	if !target_dir.exists() || !target_dir.is_dir() {
		return Err(anyhow::anyhow!(
			"Target directory '{}' not found at {}",
			target_name,
			target_dir.display()
		));
	}

	let component_executable = target_dir.join(component_name);
	if is_executable(&component_executable) {
		return run_executable(&component_executable, project_path);
	}

	if let Some(executable) = find_executable_in_dir(&target_dir, Some(component_name)) {
		return run_executable(&executable, project_path);
	}

	Err(anyhow::anyhow!(
		"Component executable '{}' not found in target directory {}",
		component_name,
		target_dir.display()
	))
}

fn run_executable(executable_path: &PathBuf, project_path: &PathBuf) -> Result<()> {
	execute_binary(executable_path, project_path)
}

fn run_main_executable(project_path: &PathBuf) -> Result<()> {
	let possible_names = vec![
		project_path.file_name().unwrap().to_string_lossy().to_string(),
		"main".to_string(),
		"app".to_string(),
		"bin".to_string(),
	];

	for name in possible_names {
		let executable_path = project_path.join(&name);
		if executable_path.exists() && executable_path.is_file() {
			log::info!("Found executable: {}", executable_path.display());
			return run_target(project_path, &name);
		}
	}

	let forge_out = project_path.join("forge-out");
	if forge_out.exists() {
		for entry in std::fs::read_dir(&forge_out)? {
			let entry = entry?;
			let path = entry.path();
			if path.is_dir() && path.file_name().unwrap().to_string_lossy().contains("unknown-linux-gnu") {
				let debug_dir = path.join("debug");
				if let Some(executable) = find_executable_in_dir(&debug_dir, None) {
					let name = executable.file_name().unwrap().to_string_lossy().to_string();
					log::info!("Found executable in forge-out: {}", executable.display());
					return run_target(project_path, &name);
				}
			}
		}
	}

	let target_dirs = vec![
		project_path.join("target").join("debug"),
		project_path.join("target").join("release"),
	];

	for target_dir in target_dirs {
		if let Some(executable) = find_executable_in_dir(&target_dir, None) {
			log::info!("Found executable in target directory: {}", executable.display());
			return run_target(project_path, &executable.file_name().unwrap().to_string_lossy());
		}
	}

	Err(anyhow::anyhow!(
		"No executable found to run. Please specify a target with --target or ensure there's an executable in the project root or target directory."
	))
}

fn clean_project(project_path: &Path) -> Result<()> {
	let forge_out_path = project_path.join("forge-out");

	if forge_out_path.exists() {
		log::info!("Removing forge-out directory: {}", forge_out_path.display());
		std::fs::remove_dir_all(&forge_out_path)?;
	} else {
		log::info!("No forge-out directory found to clean");
	}

	let target_path = project_path.join("target");
	if target_path.exists() {
		log::info!("Removing target directory: {}", target_path.display());
		std::fs::remove_dir_all(&target_path)?;
	}

	Ok(())
}


#[cfg(unix)]
fn set_executable_permissions(path: &PathBuf) -> Result<()> {
	use std::os::unix::fs::PermissionsExt;
	let mut perms = std::fs::metadata(path)?.permissions();
	perms.set_mode(0o755);
	std::fs::set_permissions(path, perms)?;
	Ok(())
}

fn is_executable(path: &PathBuf) -> bool {
	if !path.is_file() {
		return false;
	}

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		if let Ok(metadata) = std::fs::metadata(path) {
			return metadata.permissions().mode() & 0o111 != 0;
		}
	}

	#[cfg(not(unix))]
	{
		return path.extension().map_or(true, |ext| ext == "exe");
	}

	false
}

fn execute_binary(executable_path: &PathBuf, project_path: &PathBuf) -> Result<()> {
	#[cfg(unix)]
	set_executable_permissions(executable_path)?;

	log::info!("Executing: {}", executable_path.display());
	let output = Command::new(executable_path).current_dir(project_path).output()?;

	// Store stdout/stderr for test caching
	let stdout = String::from_utf8_lossy(&output.stdout).to_string();
	let stderr = String::from_utf8_lossy(&output.stderr).to_string();

	if let Ok(mut s) = last_test_stdout.lock() {
		*s = stdout.clone();
	}
	if let Ok(mut s) = last_test_stderr.lock() {
		*s = stderr.clone();
	}

	if !output.status.success() {
		return Err(anyhow::anyhow!(
			"Executable failed with exit code {:?}\nSTDOUT:\n{}\n\nSTDERR:\n{}",
			output.status.code(),
			stdout,
			stderr
		));
	}

	let stdout = String::from_utf8_lossy(&output.stdout);
	if !stdout.is_empty() {
		print!("{}", stdout);
	}

	Ok(())
}


fn find_executable_in_dir(dir: &PathBuf, name_pattern: Option<&str>) -> Option<PathBuf> {
	if !dir.exists() || !dir.is_dir() {
		return None;
	}

	let read_dir = match std::fs::read_dir(dir) {
		Ok(rd) => rd,
		Err(_) => return None,
	};

	for entry in read_dir {
		let entry = match entry {
			Ok(e) => e,
			Err(_) => continue,
		};

		let path = entry.path();
		if !path.is_file() {
			continue;
		}

		let filename = match path.file_name().and_then(|n| n.to_str()) {
			Some(name) => name,
			None => continue,
		};

		if let Some(pattern) = name_pattern
			&& !filename.starts_with(pattern)
		{
			continue;
		}

		if is_executable(&path) {
			return Some(path);
		}
	}

	None
}


fn handle_cache_command(action: CacheCommand, project_path: &Path) -> Result<()> {
	let cas_dir = project_path.join("forge-out").join("cas");
	let _db_path = cas_dir.join("cache.db");
	let _legacy_cache_path = project_path.join("forge-out").join("cache.json");

	match action {
		CacheCommand::Stats => {
			let sqlite_path = cas_dir.join("cache.db");
			if sqlite_path.exists() {
				let db = CacheDb::new(&sqlite_path)?;
				let stats = db.get_stats()?;
				println!("Cache Statistics:");
				println!("  Total files: {}", stats.total_files);
				println!(
					"  Total size: {} bytes ({} MB)",
					stats.total_size,
					stats.total_size / (1024 * 1024)
				);
				if let Some(oldest) = stats.oldest_timestamp {
					println!("  Oldest artifact: {}", oldest);
				}
				if let Some(newest) = stats.newest_timestamp {
					println!("  Newest access: {}", newest);
				}
			} else {
				// Fall back to scanning CAS directory
				let mut total_size = 0u64;
				let mut total_files = 0u64;
				if let Ok(entries) = std::fs::read_dir(&cas_dir) {
					for entry in entries.flatten() {
						if entry.path().is_file() {
							if let Ok(meta) = std::fs::metadata(entry.path()) {
								total_size += meta.len();
								total_files += 1;
							}
						}
					}
				}
				println!("Cache Statistics (scanning CAS):");
				println!("  Total files: {}", total_files);
				println!("  Total size: {} bytes ({} MB)", total_size, total_size / (1024 * 1024));
			}
		}
		CacheCommand::List { limit } => {
			let sqlite_path = cas_dir.join("cache.db");
			if sqlite_path.exists() {
				let db = CacheDb::new(&sqlite_path)?;
				let artifacts = db.list_artifacts(limit)?;
				println!("Recent artifacts in cache:");
				for artifact in artifacts {
					println!("  {} ({} bytes)", artifact.hash[..16].to_string(), artifact.size);
				}
			} else {
				println!("No SQLite cache found. Use 'forge build' to create one.");
			}
		}
		CacheCommand::Prune { max_size, older_than } => {
			let max_size_bytes = max_size
				.as_ref()
				.and_then(|s| parse_size(s))
				.unwrap_or(10 * 1024 * 1024 * 1024);
			let max_age_days = older_than.unwrap_or(30);
			let max_age = std::time::Duration::from_secs(max_age_days * 24 * 60 * 60);

			let gc = CacheGC::new(max_size_bytes, max_age, 100);
			let result: GCResult = gc.run(&cas_dir);

			println!("Cache prune complete:");
			println!("  Files removed: {}", result.files_removed);
			println!(
				"  Space freed: {} bytes ({} MB)",
				result.freed_size,
				result.freed_size / (1024 * 1024)
			);
		}
		CacheCommand::Clean => {
			let cache_path = project_path.join("forge-out");
			println!("Cleaning cache at: {}", cache_path.display());
			if cache_path.exists() {
				std::fs::remove_dir_all(&cache_path)?;
			}
			println!("Cache cleaned successfully!");
		}
	}

	Ok(())
}

fn handle_deps_command(action: DepsCommand) -> Result<()> {
	let package_manager = crate::package::PackageManager::new();

	match action {
		DepsCommand::List => {
			let packages = package_manager.list_local();
			if packages.is_empty() {
				println!("No local packages installed.");
			} else {
				println!("Local packages:");
				for pkg in packages {
					println!("  {}@{}", pkg.name, pkg.version);
				}
			}
		}
		DepsCommand::Sync => {
			println!("Syncing dependencies...");
			println!("Note: Dependency sync not yet fully implemented.");
			println!("Package directory: {}", package_manager.packages_dir().display());
		}
	}

	Ok(())
}

fn handle_toolchain_command(
	action: ToolchainCommand,
	project_path: &Path,
	verbosity: clap_verbosity_flag::Verbosity,
	cli_mode: Option<crate::hermetic::PolicyMode>,
	trace_access: bool,
	why_non_hermetic: bool,
) -> Result<()> {
	let config = crate::config::Config {
		verbosity: crate::config::VerbosityWrapper(verbosity),
		target_filters: vec![],
		component_filters: vec![],
		test_mode: false,
		profile: None,
	};
	let mut project = crate::project::Project::new(
		project_path.to_path_buf(),
		config,
		cli_mode,
		trace_access,
		why_non_hermetic,
	)?;

	match action {
		ToolchainCommand::Sync => {
			log::info!("Syncing toolchains for project at: {}", project_path.display());
			let script = r#"
                local list = forge.toolchain.list()
                if #list == 0 then
                    print("No toolchains configured in FORGE_ROOT")
                    return
                end
                for _, spec in ipairs(list) do
                    print("Syncing toolchain: " .. spec.name .. " (version: " .. (spec.version or "default") .. ")")
                    local ok, info = pcall(function() return forge.toolchain.sync(spec.name, spec) end)
                    if ok and info.available then
                        print("  ✓ Available at: " .. info.path)
                    elseif not ok then
                        print("  ✗ Failed to sync: " .. tostring(info))
                    else
                        print("  ✗ Failed to sync")
                    end
                end
            "#;
			project.load_graph()?; // Initialize Lua environment
			project.lua.load(script).exec()?;
			println!("\nToolchain sync completed!");
		}
		ToolchainCommand::List => {
			let script = r#"
                local list = forge.toolchain.list()
                if #list == 0 then
                    print("No toolchains configured in FORGE_ROOT")
                    return
                end
                print(string.format("%-20s %-15s %-10s %s", "NAME", "VERSION", "FROM", "STATUS"))
                print(string.format("%-20s %-15s %-10s %s", "----", "-------", "----", "------"))
                for _, spec in ipairs(list) do
                    local info = forge.toolchain.resolve(spec.name, spec)
                    local status = forge.fs.exists(info.install_dir or "") and "Downloaded" or "Missing"
                    if spec.from == "path" then
                        status = "Local"
                    end
                    print(string.format("%-20s %-15s %-10s %s", 
                        spec.name, 
                        spec.version or "default", 
                        spec.from, 
                        status))
                end
            "#;
			project.load_graph()?;
			project.lua.load(script).exec()?;
		}
		ToolchainCommand::Verify => {
			println!("Not yet implemented");
		}
	}
	Ok(())
}

fn parse_size(s: &str) -> Option<u64> {
	let s = s.to_uppercase();
	let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
	let num: u64 = num_str.parse().ok()?;
	let multiplier: u64 = match unit {
		"B" => 1,
		"K" => 1024,
		"M" => 1024 * 1024,
		"G" => 1024 * 1024 * 1024,
		_ => return None,
	};
	Some(num * multiplier)
}
