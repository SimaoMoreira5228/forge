use std::path::PathBuf;
use std::process::Command as Process;

use clap::{Parser, Subcommand};
use forge_diagnostics::ForgeDiagnostic;
use forge_engine::Engine;

#[derive(Parser)]
#[command(name = "forge", version, about = "A hermetic, content-addressed build system")]
struct Cli {
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	Init,
	Build {
		#[arg(long, default_value = "debug")]
		profile: String,
	},
	Run {
		name: String,
		#[arg(long, default_value = "debug")]
		profile: String,
		args: Vec<String>,
	},
	Test {
		#[arg(long, default_value = "debug")]
		profile: String,
	},
	Query {
		#[command(subcommand)]
		what: QueryCommand,
	},
	Graph {
		#[arg(long, default_value = "dot")]
		output: String,
	},
	Clean {
		#[arg(long)]
		expunge: bool,
	},
	Toolchains,
}

#[derive(Subcommand)]
enum QueryCommand {
	Deps {
		target: String,
	},
	Rdeps {
		target: String,
	},
}

fn main() {
	if let Err(e) = dispatch() {
		eprintln!("{e}");
		std::process::exit(1);
	}
}

fn dispatch() -> Result<(), ForgeDiagnostic> {
	let cli = Cli::parse();
	let workspace: PathBuf = std::env::current_dir().expect("cwd");

	match cli.command {
		Command::Init => init(&workspace),
		Command::Build { profile } => {
			let outcome = Engine::open(&workspace).build(&profile)?;
			println!(
				"build ok: {} actions ({} executed, {} cache hits, {} tests served from cache)",
				outcome.executed + outcome.cache_hits + outcome.test_cache_hits,
				outcome.executed,
				outcome.cache_hits,
				outcome.test_cache_hits
			);
			Ok(())
		}
		Command::Run { name, profile, args } => run_target(&workspace, &name, &profile, &args),
		Command::Test { profile } => {
			let outcome = Engine::open(&workspace).test(&profile)?;
			println!(
				"tests ok: {} executed, {} served from cache",
				outcome.executed, outcome.test_cache_hits
			);
			Ok(())
		}
		Command::Query { what } => query(&workspace, what),
		Command::Graph { output } => graph(&workspace, &output),
		Command::Clean { expunge } => {
			Engine::open(&workspace).clean(expunge)?;
			println!("clean");
			Ok(())
		}
		Command::Toolchains => toolchains(&workspace),
	}
}

fn init(workspace: &std::path::Path) -> Result<(), ForgeDiagnostic> {
	let root = workspace.join("FORGE_ROOT");
	if !root.exists() {
		std::fs::write(
            &root,
            "[project]\nname = \"myproject\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
        )
        .map_err(|e| ForgeDiagnostic::error(8, format!("{e}")))?;
	}
	let toml = workspace.join("FORGE.toml");
	if !toml.exists() {
		std::fs::write(&toml, "[binary.hello]\nsrcs = [\"src/main.c\"]\ncompiler = \"gcc\"\n")
			.map_err(|e| ForgeDiagnostic::error(8, format!("{e}")))?;
	}
	let src = workspace.join("src/main.c");
	if !src.exists() {
		std::fs::create_dir_all(src.parent().expect("parent")).map_err(|e| ForgeDiagnostic::error(8, format!("{e}")))?;
		std::fs::write(
			&src,
			"#include <stdio.h>\n\nint main(void) {\n    printf(\"hello forge\\n\");\n}\n",
		)
		.map_err(|e| ForgeDiagnostic::error(8, format!("{e}")))?;
	}
	println!("initialized workspace at {}", workspace.display());
	Ok(())
}

fn run_target(workspace: &std::path::Path, name: &str, profile: &str, args: &[String]) -> Result<(), ForgeDiagnostic> {
	let engine = Engine::open(workspace);
	let outcome = engine.build(profile)?;
	let path = outcome
		.binaries
		.iter()
		.find(|(label, _)| label.ends_with(&format!(":{name}")))
		.map(|(_, p)| p.clone())
		.ok_or_else(|| ForgeDiagnostic::error(2, format!("no binary named `{name}`")))?;
	let status = Process::new(&path)
		.args(args)
		.current_dir(workspace)
		.status()
		.map_err(|e| ForgeDiagnostic::error(6, format!("cannot launch `{}`: {e}", path.display())))?;
	if !status.success() {
		std::process::exit(status.code().unwrap_or(1));
	}
	Ok(())
}

fn load_graph(workspace: &std::path::Path) -> Result<forge_core::BuildGraph, ForgeDiagnostic> {
	let config = forge_script::WorkspaceConfig::load(workspace)?;
	let packages = forge_script::discover_packages(workspace, &config.discovery)?;
	let (graph, _) = forge_script::load_workspace(workspace, &packages)?;
	graph.check_visibility().map_err(|mut errs| errs.swap_remove(0))?;
	Ok(graph)
}

fn query(workspace: &std::path::Path, what: QueryCommand) -> Result<(), ForgeDiagnostic> {
	let graph = load_graph(workspace)?;
	match what {
		QueryCommand::Deps { target } => {
			let id = resolve(&graph, &target)?;
			for dep in graph.transitive_dependencies(id) {
				println!("{}", graph.component(dep).label);
			}
		}
		QueryCommand::Rdeps { target } => {
			let id = resolve(&graph, &target)?;
			for dependent in graph.transitive_dependents(id) {
				println!("{}", graph.component(dependent).label);
			}
		}
	}
	Ok(())
}

fn resolve(graph: &forge_core::BuildGraph, target: &str) -> Result<forge_core::ComponentId, ForgeDiagnostic> {
	let label = forge_core::Label::parse(target, "").or_else(|_| forge_core::Label::parse(target, "."))?;
	graph.get(&label).ok_or_else(|| {
		let names: Vec<&str> = graph.labels().map(|l| l.name()).collect();
		let suggestion = forge_diagnostics::suggest::closest(label.name(), names);
		let d = ForgeDiagnostic::error(2, format!("unknown target `{target}`"));
		match suggestion {
			Some(s) => d.with_help(format!("did you mean `{s}`?")),
			None => d,
		}
	})
}

fn graph(workspace: &std::path::Path, output: &str) -> Result<(), ForgeDiagnostic> {
	let g = load_graph(workspace)?;
	match output {
		"dot" => print!("{}", g.output_dot()),
		_ => return Err(ForgeDiagnostic::error(103, format!("unknown graph format `{output}`")).with_help("supported: dot")),
	}
	Ok(())
}

fn toolchains(workspace: &std::path::Path) -> Result<(), ForgeDiagnostic> {
	let config = forge_script::WorkspaceConfig::load(workspace)?;
	let store = forge_engine::toolchain::ToolchainStore::new(workspace, config);
	for (name, selection) in store.configured() {
		match store.resolve(name, selection) {
			Ok(resolved) => println!("{name}: ready at {}", resolved.bin_dir.display()),
			Err(e) => println!("{name}: not ready ({e})"),
		}
	}
	Ok(())
}
