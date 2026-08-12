use std::path::Path;
use std::time::Duration;

use forge_diagnostics::ForgeDiagnostic;
use forge_engine::Engine;
use notify::Watcher;

const DEBOUNCE: Duration = Duration::from_millis(150);

pub struct WatchScope {
	pub package_dirs: Vec<std::path::PathBuf>,
	pub excludes: Vec<String>,
}

pub fn watch(workspace: &Path, scope: WatchScope, profile: &str, run_tests: bool) -> Result<(), ForgeDiagnostic> {
	let engine = Engine::open(workspace);
	rebuild(&engine, profile, run_tests, 0)?;

	let mut gitignore = ignore::gitignore::GitignoreBuilder::new(workspace);
	if workspace.join(".gitignore").is_file()
		&& let Some(err) = gitignore.add(workspace.join(".gitignore"))
	{
		return Err(ForgeDiagnostic::error(8, format!("gitignore: {err}")));
	}
	let gitignore = gitignore
		.build()
		.map_err(|e| ForgeDiagnostic::error(8, format!("gitignore: {e}")))?;

	let (sender, receiver) = std::sync::mpsc::channel();
	let excludes = scope.excludes.clone();
	let mut watcher = notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
		if let Ok(event) = event {
			let worth_rebuilding = matches!(
				event.kind,
				notify::EventKind::Modify(_) | notify::EventKind::Create(_) | notify::EventKind::Remove(_)
			);
			if worth_rebuilding && event.paths.iter().any(|p| relevant(p, &excludes, &gitignore)) {
				let _ = sender.send(());
			}
		}
	})
	.map_err(|e| ForgeDiagnostic::error(8, format!("watcher: {e}")))?;

	for dir in &scope.package_dirs {
		watcher
			.watch(dir, notify::RecursiveMode::Recursive)
			.map_err(|e| ForgeDiagnostic::error(8, format!("watch {}: {e}", dir.display())))?;
	}

	println!("watching {} package(s) — Ctrl+C to stop", scope.package_dirs.len());
	let mut generation = 1;
	loop {
		if receiver.recv().is_err() {
			return Ok(());
		}
		while receiver.recv_timeout(DEBOUNCE).is_ok() {}
		generation += 1;
		rebuild(&engine, profile, run_tests, generation)?;
	}
}

fn rebuild(engine: &Engine, profile: &str, run_tests: bool, generation: usize) -> Result<(), ForgeDiagnostic> {
	println!("\n=== [{}] {} ===", generation, chrono_stamp());
	let started = std::time::Instant::now();
	let result = if run_tests {
		engine.test(profile, None)
	} else {
		engine.build(profile, None)
	};
	match result {
		Ok(outcome) => println!(
			"done in {:.2}s ({} executed, {} cache hits, {} tests cached)",
			started.elapsed().as_secs_f32(),
			outcome.executed,
			outcome.cache_hits,
			outcome.test_cache_hits,
		),
		Err(e) => eprintln!("{e}"),
	}
	Ok(())
}

fn relevant(path: &Path, excludes: &[String], gitignore: &ignore::gitignore::Gitignore) -> bool {
	path.components().all(|component| match component {
		std::path::Component::Normal(part) => {
			let part = part.to_string_lossy();
			!matches!(part.as_ref(), "forge-out" | ".forge" | ".git")
				&& !excludes.iter().any(|excluded| excluded == &part)
				&& !matches!(gitignore.matched(path, path.is_dir()), ignore::Match::Ignore(_))
		}
		_ => true,
	})
}

fn chrono_stamp() -> String {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0)
		.to_string()
}
