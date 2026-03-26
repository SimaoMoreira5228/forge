use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use notify::{Watcher, RecursiveMode, EventKind};
use crate::{config::Config, hermetic::HermeticPolicy, project::Project};

pub fn watch_project(
	project_path: PathBuf,
	config: Config,
	cli_mode: Option<crate::hermetic::PolicyMode>,
	trace_access: bool,
	why_non_hermetic: bool,
) -> anyhow::Result<()> {
	log::info!("Starting initial build before entering watch mode...");

	let mut project = match Project::new(
		project_path.clone(),
		config.clone(),
		cli_mode,
		trace_access,
		why_non_hermetic,
	) {
		Ok(mut proj) => {
			let start = Instant::now();
			if let Err(e) = proj.run() {
				log::error!("Initial build failed: {}", e);
			} else {
				log::info!("Initial build succeeded in {:?}", start.elapsed());
			}
			proj
		}
		Err(e) => {
			log::error!("Failed to initialize project: {}", e);
			return Err(e.into());
		}
	};

	let (tx, rx) = channel();
	let mut watcher = notify::recommended_watcher(move |res| {
		let _ = tx.send(res);
	})?;

	watcher.watch(&project_path, RecursiveMode::Recursive)?;
	log::info!("Watching for changes in {}", project_path.display());

	let debounce_idle = Duration::from_millis(150);

	loop {
		match rx.recv() {
			Ok(res) => match res {
				Ok(event) => {
					let is_valid = |ev: &notify::Event| -> bool {
						match ev.kind {
							EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
								!ev.paths.iter().any(|p| {
									let s = p.to_string_lossy();
									s.contains("/.forge/")
										|| s.contains("/forge-out/")
										|| s.contains("/target/")
										|| s.contains("/.git/")
										|| s.ends_with("/.forge")
										|| s.ends_with("/forge-out")
										|| s.ends_with("/target")
										|| s.ends_with("/.git")
								})
							}
							_ => false,
						}
					};

					if is_valid(&event) {
						// Collect all valid events during debounce
						let mut events = vec![event];
						loop {
							match rx.recv_timeout(debounce_idle) {
								Ok(Ok(ev)) => {
									if is_valid(&ev) {
										events.push(ev);
										continue;
									}
								}
								Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
								_ => continue,
							}
						}

						// Discard whatever is left just in case
						while let Ok(_) = rx.try_recv() {}

						let affects_build_logic = events.iter().any(|ev| {
							ev.paths.iter().any(|p| {
								let filename = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
								filename == "FORGE" || filename == "FORGE_ROOT"
							})
						});

						let forge_root_changed = events.iter().any(|ev| {
							ev.paths.iter().any(|p| {
								let filename = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
								filename == "FORGE_ROOT"
							})
						});

						log::info!("\n=== Filesystem settled, rebuilding ===");
						let start = Instant::now();

						if forge_root_changed {
							log::info!("FORGE_ROOT changed, re-initializing project...");
							match Project::new(
								project_path.clone(),
								config.clone(),
								cli_mode,
								trace_access,
								why_non_hermetic,
							) {
								Ok(mut new_proj) => {
									project = new_proj;
									if let Err(e) = project.run() {
										log::error!("Build failed: {}", e);
									} else {
										log::info!("Build succeeded in {:?}", start.elapsed());
									}
								}
								Err(e) => log::error!("Failed to re-initialize project: {}", e),
							}
						} else if affects_build_logic {
							log::info!("FORGE files changed, reloading graph...");
							if let Err(e) = project.run() {
								log::error!("Build failed: {}", e);
							} else {
								log::info!("Build succeeded (with graph reload) in {:?}", start.elapsed());
							}
						} else {
							log::info!("Source files changed, re-running execution...");
							// Fast path: skip load_graph, just execute
							// We need to call execute_build_graph and optionally execute_tests
							// Since Project::run is: load_graph + execute_build_graph + execute_tests,
							// we'll implement a partial_run or just call them here.
							
							let res = (|| -> anyhow::Result<()> {
								// We need to re-scan for stale rules because inputs changed
								// execute_build_graph does this via needs_rebuild
								project.execute_build_graph()?;
								if project.config.test_mode {
									project.execute_tests()?;
								}
								Ok(())
							})();

							if let Err(e) = res {
								log::error!("Build failed: {}", e);
							} else {
								log::info!("Build succeeded (fast path) in {:?}", start.elapsed());
							}
						}

						log::info!("Watching for changes...");
					}
				}
				Err(e) => log::error!("watch error: {:?}", e),
			},
			Err(_) => break, // Channel disconnected
		}
	}

	Ok(())
}
