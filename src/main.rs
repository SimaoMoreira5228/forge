use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod cache;
mod config;
mod error;
mod forge_root_config;
mod lua_api;
mod project;

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

fn init_forge_root(project_path: &PathBuf, name: Option<String>, force: bool) -> Result<()> {
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

fn migrate_to_forge_root(project_path: &PathBuf, force: bool) -> Result<()> {
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
			if target_dir.exists() && target_dir.is_dir() {
				for entry in std::fs::read_dir(&target_dir)? {
					let entry = entry?;
					let path = entry.path();
					if path.is_file() {
						#[cfg(unix)]
						{
							use std::os::unix::fs::PermissionsExt;
							if let Ok(metadata) = std::fs::metadata(&path) {
								if metadata.permissions().mode() & 0o111 != 0 {
									target_path = path;
									break;
								}
							}
						}
						#[cfg(not(unix))]
						{
							if path.extension().map_or(true, |ext| ext == "exe") {
								target_path = path;
								break;
							}
						}
					}
				}
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

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mut perms = std::fs::metadata(&target_path)?.permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(&target_path, perms)?;
	}

	log::info!("Executing target: {}", target_path.display());
	let output = std::process::Command::new(&target_path).current_dir(project_path).output()?;

	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		let stdout = String::from_utf8_lossy(&output.stdout);
		return Err(anyhow::anyhow!(
			"Target execution failed with exit code {:?}\nSTDOUT:\n{}\n\nSTDERR:\n{}",
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
	if component_executable.exists() && component_executable.is_file() {
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			if let Ok(metadata) = std::fs::metadata(&component_executable) {
				if metadata.permissions().mode() & 0o111 != 0 {
					return run_executable(&component_executable, project_path);
				}
			}
		}
		#[cfg(not(unix))]
		{
			if component_executable.extension().map_or(true, |ext| ext == "exe") {
				return run_executable(&component_executable, project_path);
			}
		}
	}

	for entry in std::fs::read_dir(&target_dir)? {
		let entry = entry?;
		let path = entry.path();
		if path.is_file() {
			if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
				if filename.starts_with(component_name) {
					#[cfg(unix)]
					{
						use std::os::unix::fs::PermissionsExt;
						if let Ok(metadata) = std::fs::metadata(&path) {
							if metadata.permissions().mode() & 0o111 != 0 {
								return run_executable(&path, project_path);
							}
						}
					}
					#[cfg(not(unix))]
					{
						if path.extension().map_or(true, |ext| ext == "exe") {
							return run_executable(&path, project_path);
						}
					}
				}
			}
		}
	}

	return Err(anyhow::anyhow!(
		"Component executable '{}' not found in target directory {}",
		component_name,
		target_dir.display()
	));
}

fn run_executable(executable_path: &PathBuf, project_path: &PathBuf) -> Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mut perms = std::fs::metadata(executable_path)?.permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(executable_path, perms)?;
	}

	log::info!("Executing: {}", executable_path.display());
	let output = std::process::Command::new(executable_path)
		.current_dir(project_path)
		.output()?;

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
				if debug_dir.exists() {
					for debug_entry in std::fs::read_dir(&debug_dir)? {
						let debug_entry = debug_entry?;
						let debug_path = debug_entry.path();
						if debug_path.is_file() {
							#[cfg(unix)]
							{
								use std::os::unix::fs::PermissionsExt;
								if let Ok(metadata) = std::fs::metadata(&debug_path) {
									if metadata.permissions().mode() & 0o111 != 0 {
										let name = debug_path.file_name().unwrap().to_string_lossy().to_string();
										log::info!("Found executable in forge-out: {}", debug_path.display());
										return run_target(project_path, &name);
									}
								}
							}
							#[cfg(not(unix))]
							{
								if debug_path.extension().is_none() {
									let name = debug_path.file_name().unwrap().to_string_lossy().to_string();
									log::info!("Found potential executable in forge-out: {}", debug_path.display());
									return run_target(project_path, &name);
								}
							}
						}
					}
				}
			}
		}
	}

	let target_dirs = vec![
		project_path.join("target").join("debug"),
		project_path.join("target").join("release"),
	];

	for target_dir in target_dirs {
		if target_dir.exists() {
			for entry in std::fs::read_dir(&target_dir)? {
				let entry = entry?;
				let path = entry.path();
				if path.is_file() {
					#[cfg(unix)]
					{
						use std::os::unix::fs::PermissionsExt;
						if let Ok(metadata) = std::fs::metadata(&path) {
							if metadata.permissions().mode() & 0o111 != 0 {
								log::info!("Found executable in target directory: {}", path.display());
								return run_target(project_path, &path.file_name().unwrap().to_string_lossy());
							}
						}
					}
					#[cfg(not(unix))]
					{
						if path.extension().is_none() {
							log::info!("Found potential executable in target directory: {}", path.display());
							return run_target(project_path, &path.file_name().unwrap().to_string_lossy());
						}
					}
				}
			}
		}
	}

	Err(anyhow::anyhow!(
		"No executable found to run. Please specify a target with --target or ensure there's an executable in the project root or target directory."
	))
}

fn clean_project(project_path: &PathBuf) -> Result<()> {
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
