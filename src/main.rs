use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

mod cache;
mod config;
mod error;
mod forge_root_config;
mod lua_api;
mod project;

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
	},

	Clean,

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
}

fn main() -> Result<()> {
	let cli = Cli::parse();

	env_logger::Builder::new().filter_level(cli.verbose.log_level_filter()).init();

	let project_path = std::fs::canonicalize(&cli.project)?;

	match cli.command {
		Some(Commands::Build { target, component }) => {
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
			};

			log::info!("Building project at: {}", project_path.display());
			if !config.target_filters.is_empty() {
				log::info!("Target filters: {}", config.target_filters.join(", "));
			}
			if !config.component_filters.is_empty() {
				log::info!("Component filters: {}", config.component_filters.join(", "));
			}

			let mut project = project::Project::new(project_path, config)?;
			project.run()?;

			println!("\nBuild completed successfully!");
		}
		Some(Commands::Run { target, component }) => {
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: vec![],
				component_filters: if let Some(ref comp) = component {
					vec![comp.clone()]
				} else {
					vec![]
				},
				test_mode: false,
			};

			log::info!("Building and running project at: {}", project_path.display());

			let mut project = project::Project::new(project_path.clone(), config)?;
			project.run()?;

			if let Some(target_name) = target {
				if let Some(comp) = component {
					log::info!("Running component '{}' with target: {}", comp, target_name);
					run_component_target(&project_path, &comp, &target_name)?;
				} else {
					log::info!("Running target: {}", target_name);
					run_target(&project_path, &target_name)?;
				}
			} else {
				run_main_executable(&project_path)?;
			}

			println!("\nBuild and run completed successfully!");
		}
		Some(Commands::Test { target, component }) => {
			let config = config::Config {
				verbosity: config::VerbosityWrapper(cli.verbose),
				target_filters: vec![target.clone()],
				component_filters: if let Some(ref comp) = component {
					vec![comp.clone()]
				} else {
					vec![]
				},
				test_mode: true,
			};

			log::info!("Building and testing project at: {}", project_path.display());
			log::info!("Test target: {}", target);
			if let Some(ref comp) = component {
				log::info!("Test component: {}", comp);
			}

			let mut project = project::Project::new(project_path.clone(), config)?;
			project.run()?;

			if let Some(comp) = component {
				log::info!("Running test component '{}' with target: {}", comp, target);
				run_component_target_test_mode(&project_path, &comp, &target)?;
			} else {
				log::info!("Running test target: {}", target);
				run_target_test_mode(&project_path, &target)?;
			}

			println!("\nTest completed successfully!");
		}
		Some(Commands::Clean) => {
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
			};

			log::info!("Building project at: {}", project_path.display());
			log::info!("Targets: {}", config.target_filters.join(", "));

			let mut project = project::Project::new(project_path, config)?;
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

fn run_target_test_mode(project_path: &PathBuf, target_name: &str) -> Result<()> {
	let forge_out = project_path.join("forge-out");
	if !forge_out.exists() {
		return Err(anyhow::anyhow!("forge-out directory not found at {}", forge_out.display()));
	}

	let target_dir = forge_out.join(target_name);
	let test_executables = find_all_test_executables_in_dir(&target_dir);

	if test_executables.is_empty() {
		return Err(anyhow::anyhow!(
			"No test executables found for target '{}' (looking for binaries with '_test' suffix)",
			target_name
		));
	}

	println!("Running {} test(s) for target '{}'...", test_executables.len(), target_name);

	let mut all_passed = true;
	for test_executable in test_executables {
		println!(
			"\n=== Running test: {} ===",
			test_executable.file_name().unwrap().to_str().unwrap()
		);
		match run_executable(&test_executable, project_path) {
			Ok(_) => {}
			Err(e) => {
				eprintln!("Test failed: {}", e);
				all_passed = false;
			}
		}
	}

	if !all_passed {
		return Err(anyhow::anyhow!("One or more tests failed"));
	}

	Ok(())
}

fn run_component_target_test_mode(project_path: &PathBuf, component_name: &str, target_name: &str) -> Result<()> {
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

	if let Some(test_executable) = find_test_executable_in_dir(&target_dir, Some(component_name)) {
		return run_executable(&test_executable, project_path);
	}

	Err(anyhow::anyhow!(
		"Test component executable '{}_test' not found in target directory {} (looking for binaries with '_test' suffix)",
		component_name,
		target_dir.display()
	))
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

	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		let stdout = String::from_utf8_lossy(&output.stdout);
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

fn find_all_test_executables_in_dir(dir: &PathBuf) -> Vec<PathBuf> {
	let mut test_executables = Vec::new();

	if !dir.exists() || !dir.is_dir() {
		return test_executables;
	}

	let read_dir = match std::fs::read_dir(dir) {
		Ok(rd) => rd,
		Err(_) => return test_executables,
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

		let is_test_executable = filename.ends_with("_test") || filename.ends_with("_test.exe");
		if !is_test_executable {
			continue;
		}

		if is_executable(&path) {
			test_executables.push(path);
		}
	}

	test_executables.sort();
	test_executables
}

fn find_test_executable_in_dir(dir: &PathBuf, component_pattern: Option<&str>) -> Option<PathBuf> {
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

		let is_test_executable = filename.ends_with("_test") || filename.ends_with("_test.exe");
		if !is_test_executable {
			continue;
		}

		if let Some(pattern) = component_pattern {
			let expected_test_name = format!("{}_test", pattern);
			if !filename.starts_with(&expected_test_name) {
				continue;
			}
		}

		if is_executable(&path) {
			return Some(path);
		}
	}

	None
}
