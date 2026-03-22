use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use notify::{Watcher, RecursiveMode, EventKind};
use crate::{config::Config, hermetic::HermeticPolicy, project::Project};

pub fn watch_project(
	project_path: PathBuf,
	config: Config,
	policy: HermeticPolicy,
) -> anyhow::Result<()> {
	log::info!("Starting initial build before entering watch mode...");

	let start = Instant::now();
	match Project::new(project_path.clone(), config.clone(), policy.clone()) {
		Ok(mut proj) => {
			if let Err(e) = proj.run() {
				log::error!("Initial build failed: {}", e);
			} else {
				log::info!("Initial build succeeded in {:?}", start.elapsed());
			}
		}
		Err(e) => log::error!("Failed to initialize project: {}", e),
	}

	let (tx, rx) = channel();
	let mut watcher = notify::recommended_watcher(move |res| {
		let _ = tx.send(res);
	})?;

	watcher.watch(&project_path, RecursiveMode::Recursive)?;
	log::info!("Watching for changes in {}", project_path.display());

	let debounce_duration = Duration::from_millis(250);
	let mut last_build = Instant::now() - debounce_duration; // Allow immediate first trigger if generated

	loop {
		match rx.recv() {
			Ok(res) => match res {
				Ok(event) => {
					match event.kind {
						EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
							// Filter out standard output paths and git chatter
							let ignore = event.paths.iter().any(|p| {
								let s = p.to_string_lossy();
								s.contains("/.forge/")
									|| s.contains("/forge-out/")
									|| s.contains("/target/")
									|| s.contains("/.git/")
									|| s.ends_with("/.forge")
									|| s.ends_with("/forge-out")
									|| s.ends_with("/target")
									|| s.ends_with("/.git")
							});

							if ignore {
								continue;
							}

							let now = Instant::now();
							if now.duration_since(last_build) >= debounce_duration {
								log::info!("\n=== File changed {:?}, rebuilding ===", event.paths.first().unwrap_or(&PathBuf::new()));
								
								// Flush debounce channel
								while let Ok(_) = rx.try_recv() {}

								let start = Instant::now();
								match Project::new(project_path.clone(), config.clone(), policy.clone()) {
									Ok(mut proj) => {
										if let Err(e) = proj.run() {
											log::error!("Build failed: {}", e);
										} else {
											log::info!("Build succeeded in {:?}", start.elapsed());
										}
									}
									Err(e) => log::error!("Failed to initialize project: {}", e),
								}

								last_build = Instant::now();
								log::info!("Watching for changes...");
							}
						}
						_ => {}
					}
				}
				Err(e) => log::error!("watch error: {:?}", e),
			},
			Err(_) => break, // Channel disconnected
		}
	}

	Ok(())
}
