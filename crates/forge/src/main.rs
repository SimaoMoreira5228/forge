use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use forge_diagnostics::ForgeDiagnostic;
use forge_engine::Engine;

mod watch;

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
		target: Option<String>,
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
		target: Option<String>,
		#[arg(long, default_value = "debug")]
		profile: String,
		#[arg(long)]
		flake_report: bool,
		#[arg(long)]
		output: Option<String>,
	},
	Watch {
		#[arg(long, default_value = "debug")]
		profile: String,
		#[arg(long)]
		test: bool,
	},
	Explain {
		target: String,
		#[arg(long, default_value = "debug")]
		profile: String,
	},
	Query {
		expr: String,
		#[arg(long, value_parser = ["label", "json", "dot", "count"], default_value = "label")]
		output: String,
	},
	Stats {
		#[arg(long, default_value = "10")]
		limit: usize,
	},
	Confine,
	Coverage {
		target: Option<String>,
		#[arg(long)]
		output: Option<String>,
	},
	CompileCommands {
		#[arg(long, default_value = "debug")]
		profile: String,
		#[arg(long, default_value = "compile_commands.json")]
		output: String,
	},
	Graph {
		#[arg(long, default_value = "dot")]
		output: String,
	},
	Fmt {
		#[arg(long)]
		check: bool,
	},
	Verify {
		#[arg(long, default_value = "forge-out/forge.proof")]
		proof: PathBuf,
	},
	Replay {
		proof: PathBuf,
		target: Option<String>,
		#[arg(long, default_value = "debug")]
		profile: String,
	},
	TimeTravel {
		#[arg(long)]
		to: Option<String>,
		#[arg(long)]
		proof: Option<PathBuf>,
	},
	Clean {
		#[arg(long)]
		cache: bool,
		#[arg(long)]
		test: bool,
	},
	Cache {
		#[command(subcommand)]
		action: CacheAction,
	},
	Toolchains {
		#[command(subcommand)]
		action: ToolchainAction,
	},
	Deps {
		#[command(subcommand)]
		action: DepsAction,
	},
	Worker,
}

#[derive(Subcommand)]
enum CacheAction {
	Clean {
		#[arg(long)]
		global: bool,
	},
}

#[derive(Subcommand)]
enum ToolchainAction {
	List,
	Sync {
		name: Option<String>,
	},
	Verify {
		name: Option<String>,
	},
}

#[derive(Subcommand)]
enum DepsAction {
	Lock,
	Sync,
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

		Command::Build { target, profile } => {
			let outcome = Engine::open(&workspace).build(&profile, target.as_deref())?;
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

		Command::Test {
			target,
			profile,
			flake_report,
			output,
		} => {
			let engine = Engine::open(&workspace);
			let outcome_result = engine.test(&profile, target.as_deref());

			if let Some(spec) = &output {
				let path = match spec.strip_prefix("junit:") {
					Some(path) => path,
					None => {
						return Err(ForgeDiagnostic::error(103, format!("unknown report format `{spec}`"))
							.with_help("supported: junit:<file.xml>"));
					}
				};
				let db = forge_engine::db::CacheDb::open(&workspace.join("forge-out"))?;
				let cases: Vec<forge_engine::junit::JunitCase> = db
					.latest_test_rows()
					.into_iter()
					.map(|(component, verdict, duration_ms, stderr)| forge_engine::junit::JunitCase {
						component,
						verdict,
						duration_ms,
						stderr,
					})
					.collect();
				std::fs::write(path, forge_engine::junit::junit_xml(&cases))
					.map_err(|e| ForgeDiagnostic::error(8, format!("{path}: {e}")))?;
			}

			let outcome = outcome_result?;
			println!(
				"tests ok: {} executed, {} served from cache",
				outcome.executed, outcome.test_cache_hits
			);
			if flake_report {
				let db = forge_engine::db::CacheDb::open(&workspace.join("forge-out"))?;
				println!("\ncomponent                          passed/total   rate");
				for (component, passed, total) in db.flake_report() {
					let flaky = passed > 0 && passed < total;
					let rate = if total > 0 {
						passed as f64 / total as f64 * 100.0
					} else {
						100.0
					};
					let marker = if flaky { "  FLAKY" } else { "" };
					println!("{component:<34} {passed:>4}/{total:<4} {rate:5.1}%{marker}");
				}
			}
			Ok(())
		}

		Command::Watch { profile, test } => {
			let config = forge_script::WorkspaceConfig::load(&workspace)?;
			let packages = forge_script::discover_packages(&workspace, &config.discovery)?;
			let mut package_dirs: Vec<PathBuf> = packages
				.iter()
				.map(|p| p.file.parent().expect("config parent").to_path_buf())
				.collect();
			package_dirs.sort();
			package_dirs.dedup();
			let scope = watch::WatchScope {
				package_dirs,
				excludes: config.discovery.exclude.clone(),
			};
			watch::watch(&workspace, scope, &profile, test)
		}

		Command::Explain { target, profile } => {
			let explanations = Engine::open(&workspace).explain(&target, &profile)?;
			print!("{}", forge_engine::explain::render(&explanations));
			Ok(())
		}

		Command::Query { expr, output } => {
			let engine = Engine::open(&workspace);
			let (prepared, _) = engine.plan_dag("debug")?;
			let graph = prepared.graph;
			let parsed = forge_core::graph::query::parse(&expr)?;
			let ids = forge_core::graph::query::evaluate(&graph, &parsed)?;
			let format = match output.as_str() {
				"json" => forge_engine::query_output::OutputFormat::Json,
				"dot" => forge_engine::query_output::OutputFormat::Dot,
				"count" => forge_engine::query_output::OutputFormat::Count,
				_ => forge_engine::query_output::OutputFormat::Label,
			};
			print!("{}", forge_engine::query_output::format_results(&graph, &ids, format, &expr));
			Ok(())
		}

		Command::Stats { limit } => {
			let db = forge_engine::db::CacheDb::open(&workspace.join("forge-out"))?;

			let slowest = db.slowest_actions(limit);
			if !slowest.is_empty() {
				println!("slowest actions:");
				println!("{:<42} {:>6} {:>9} {:>9}", "action", "runs", "avg ms", "max ms");
				for (component, name, runs, avg, max) in &slowest {
					let label = format!("{component}:{name}");
					println!("{:<42} {runs:>6} {avg:>9.1} {max:>9}", truncate(&label, 42));
				}
				println!();
			}

			let rates = db.hit_rates();
			if !rates.is_empty() {
				println!("cache hit rates:");
				println!("{:<42} {:>5} {:>7} {:>6}", "action", "hits", "misses", "rate");
				for (component, name, hits, total) in &rates {
					let (hits, total) = (*hits, *total);
					let misses = total - hits;
					let rate = if total > 0 {
						hits as f64 / total as f64 * 100.0
					} else {
						100.0
					};
					let label = format!("{component}:{name}");
					println!("{:<42} {hits:>5} {misses:>7} {rate:>5.1}%", truncate(&label, 42));
				}
			}
			if slowest.is_empty() && rates.is_empty() {
				println!("no telemetry recorded yet — run a build first");
			}
			Ok(())
		}

		Command::Confine => {
			print!("{}", forge_engine::confine::active().renders());
			Ok(())
		}

		Command::Coverage { target, output } => Engine::open(&workspace).coverage(output.as_deref(), target.as_deref()),

		Command::CompileCommands { profile, output } => {
			let json = Engine::open(&workspace).compile_commands(&profile)?;
			let out_path = workspace.join(&output);
			std::fs::write(&out_path, json.as_bytes()).map_err(|e| ForgeDiagnostic::error(8, format!("{e}")))?;
			println!("wrote {}", out_path.display());
			Ok(())
		}

		Command::Graph { output } => {
			let engine = Engine::open(&workspace);
			let (prepared, _) = engine.plan_dag("debug")?;
			match output.as_str() {
				"dot" => print!("{}", prepared.graph.output_dot()),
				_ => {
					return Err(
						ForgeDiagnostic::error(103, format!("unknown graph format `{output}`")).with_help("supported: dot")
					);
				}
			}
			Ok(())
		}

		Command::Clean { cache, test } => {
			if !cache && !test {
				Engine::open(&workspace).clean()?;
			}
			if cache {
				let cas = workspace.join("forge-out/cas");
				if cas.exists() {
					std::fs::remove_dir_all(&cas).map_err(|e| ForgeDiagnostic::error(8, format!("clean failed: {e}")))?;
				}
				println!("removed forge-out/cas");
			}
			if test {
				forge_engine::db::CacheDb::open(&workspace.join("forge-out"))?.wipe_test_results();
				println!("cleared test verdict cache");
			}
			Ok(())
		}

		Command::Cache { action } => match action {
			CacheAction::Clean { global } => {
				let store = forge_engine::store::Store::open();
				if global {
					store.clean()?;
					println!("removed global store at {}", store.root().display());
				} else {
					let cas = workspace.join("forge-out/cas");
					if cas.exists() {
						std::fs::remove_dir_all(&cas)
							.map_err(|e| ForgeDiagnostic::error(8, format!("clean failed: {e}")))?;
					}
					println!("removed forge-out/cas");
				}
				Ok(())
			}
		},

		Command::TimeTravel { to, proof } => {
			let path = match (to, proof) {
				(_, Some(path)) => workspace.join(path),
				(Some(revision), None) => forge_engine::time_travel::revision_proof_path(&workspace, &revision)?,
				(None, None) => {
					return Err(ForgeDiagnostic::error(
						8,
						"time-travel needs `--to <commit>` or `--proof <path>`",
					));
				}
			};
			let (restored, unavailable) = Engine::open(&workspace).time_travel(&path)?;
			println!(
				"time-travel restored {restored} actions from {} ({unavailable} unavailable)",
				path.display()
			);
			Ok(())
		}

		Command::Replay { proof, target, profile } => {
			let path = workspace.join(proof);
			let (actions, divergences) = Engine::open(&workspace).replay(&profile, target.as_deref(), &path)?;
			if divergences.is_empty() {
				println!("replay reproduced {actions} actions");
				Ok(())
			} else {
				for divergence in &divergences {
					eprintln!("{}: {}", divergence.action, divergence.detail);
				}
				Err(ForgeDiagnostic::error(
					8,
					format!("replay diverged in {} of {actions} actions", divergences.len()),
				))
			}
		}

		Command::Verify { proof } => {
			let path = workspace.join(proof);
			let proof = forge_engine::proof::Proof::load(&path)?;
			let divergences = proof.verify(&workspace)?;
			if divergences.is_empty() {
				println!("proof verified: {} actions", proof.entries.len());
				Ok(())
			} else {
				for divergence in &divergences {
					if divergence.action.is_empty() {
						eprintln!("{}", divergence.detail);
					} else {
						eprintln!("{}: {}", divergence.action, divergence.detail);
					}
				}
				Err(ForgeDiagnostic::error(
					8,
					format!("proof verification failed: {} divergences", divergences.len()),
				))
			}
		}

		Command::Fmt { check } => {
			let config = forge_script::WorkspaceConfig::load(&workspace)?;
			let changed = forge_script::fmt::fmt_workspace(&workspace, &config.discovery, check)?;
			if changed.is_empty() {
				println!("all FORGE.toml files formatted");
			} else {
				for path in &changed {
					if check {
						println!("would reformat {}", path.display());
					} else {
						println!("reformatted {}", path.display());
					}
				}
				if check {
					std::process::exit(1);
				}
			}
			Ok(())
		}

		Command::Toolchains { action } => match action {
			ToolchainAction::List => toolchains(&workspace),
			ToolchainAction::Sync { name } => {
				let config = forge_script::WorkspaceConfig::load(&workspace)?;
				let store = forge_engine::toolchain::ToolchainStore::load(&workspace, config)?;
				for (name, outcome) in forge_engine::toolchain::sync::sync_all(&store, name.as_deref())? {
					match outcome {
						forge_engine::toolchain::sync::SyncOutcome::AlreadyInstalled => {
							println!("{name}: up to date");
						}
						forge_engine::toolchain::sync::SyncOutcome::Downloaded { url } => {
							println!("{name}: installed from {url}");
						}
					}
				}
				Ok(())
			}
			ToolchainAction::Verify { name } => {
				let config = forge_script::WorkspaceConfig::load(&workspace)?;
				let store = forge_engine::toolchain::ToolchainStore::load(&workspace, config)?;
				for line in forge_engine::toolchain::sync::verify_installed(&store, name.as_deref())? {
					println!("{line}");
				}
				Ok(())
			}
		},

		Command::Deps {
			action: DepsAction::Lock,
		} => {
			let path = Engine::open(&workspace).write_dependency_lock()?;
			println!("wrote {}", path.display());
			Ok(())
		}
		Command::Deps {
			action: DepsAction::Sync,
		} => {
			for package in Engine::open(&workspace).sync_dependencies()? {
				println!(
					"{}@{}: {}",
					package.name,
					package.version,
					workspace.join(package.root).display()
				);
			}
			Ok(())
		}

		Command::Worker => {
			let stdin = std::io::stdin();
			let mut stdout = std::io::stdout();
			forge_engine::worker::serve(&mut stdin.lock(), &mut stdout).map_err(|e| ForgeDiagnostic::error(8, e))
		}
	}
}

fn init(workspace: &Path) -> Result<(), ForgeDiagnostic> {
	if !workspace.join("FORGE_ROOT").exists() {
		std::fs::write(
			workspace.join("FORGE_ROOT"),
			"[project]\nname = \"myproject\"\n\n[discovery]\ninclude = [\".\"]\n\n[toolchains.gcc]\nfrom = \"path\"\npath = \"/usr\"\n",
		)
		.map_err(|e| ForgeDiagnostic::error(8, format!("{e}")))?;
	}
	if !workspace.join("FORGE.toml").exists() {
		std::fs::write(
			workspace.join("FORGE.toml"),
			"[binary.hello]\nsrcs = [\"src/main.c\"]\ncompiler = \"gcc\"\n",
		)
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

fn run_target(workspace: &PathBuf, name: &str, profile: &str, args: &[String]) -> Result<(), ForgeDiagnostic> {
	let engine = Engine::open(workspace);
	let outcome = engine.build(profile, None)?;
	let path = outcome
		.binaries
		.iter()
		.find(|(label, _)| label.ends_with(&format!(":{name}")))
		.map(|(_, p)| p.clone())
		.ok_or_else(|| ForgeDiagnostic::error(2, format!("no binary named `{name}`")))?;
	let status = std::process::Command::new(&path)
		.args(args)
		.current_dir(workspace)
		.status()
		.map_err(|e| ForgeDiagnostic::error(6, format!("cannot launch `{}`: {e}", path.display())))?;
	if !status.success() {
		std::process::exit(status.code().unwrap_or(1));
	}
	Ok(())
}

fn truncate(text: &str, max: usize) -> String {
	if text.chars().count() <= max {
		text.to_string()
	} else {
		let head: String = text.chars().take(max - 1).collect();
		format!("{head}…")
	}
}

fn toolchains(workspace: &Path) -> Result<(), ForgeDiagnostic> {
	let config = forge_script::WorkspaceConfig::load(workspace)?;
	let store = forge_engine::toolchain::ToolchainStore::load(workspace, config)?;
	for (name, selection) in store.configured() {
		match store.resolve(name, selection) {
			Ok(resolved) => println!("{name}: ready at {}", resolved.bin_dir.display()),
			Err(e) => println!("{name}: not ready ({e})"),
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn dependency_commands_have_no_lock_path_overrides() {
		assert!(Cli::try_parse_from(["forge", "deps", "lock"]).is_ok());
		assert!(Cli::try_parse_from(["forge", "deps", "sync"]).is_ok());
		assert!(Cli::try_parse_from(["forge", "deps", "lock", "--output", "other.lock"]).is_err());
		assert!(Cli::try_parse_from(["forge", "deps", "sync", "--lock", "other.lock"]).is_err());
	}
}
