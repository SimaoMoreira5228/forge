use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

mod forge_cli;

use forge_cli::run_forge;

struct SparseRegistry {
	base: String,
	stop: Arc<AtomicBool>,
}

impl SparseRegistry {
	fn serve(documents: &[(&str, String)]) -> Self {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let mut documents: BTreeMap<String, String> = documents
			.iter()
			.map(|(path, body)| ((*path).to_string(), body.clone()))
			.collect();
		documents.insert("/config.json".into(), format!(r#"{{"dl":"{base}/crates"}}"#));
		let stop = Arc::new(AtomicBool::new(false));
		let closing = stop.clone();
		std::thread::spawn(move || {
			listener.set_nonblocking(true).unwrap();
			while !closing.load(Ordering::SeqCst) {
				let (mut stream, _) = match listener.accept() {
					Ok(accepted) => accepted,
					Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
						std::thread::sleep(Duration::from_millis(2));
						continue;
					}
					Err(_) => break,
				};
				stream.set_nonblocking(false).unwrap();
				stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
				let mut request = Vec::new();
				let mut byte = [0];
				while !request.ends_with(b"\r\n\r\n") {
					match stream.read_exact(&mut byte) {
						Ok(()) => request.push(byte[0]),
						Err(_) => break,
					}
				}
				let path = String::from_utf8_lossy(&request)
					.lines()
					.next()
					.and_then(|line| line.split_whitespace().nth(1))
					.unwrap_or("")
					.to_string();
				let body = documents.get(&path).cloned();
				let (status, body) = match body {
					Some(body) => ("200 OK", body),
					None => ("404 Not Found", String::new()),
				};
				write!(
					stream,
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
					body.len()
				)
				.unwrap();
				let _ = stream.write_all(body.as_bytes());
			}
		});
		Self { base, stop }
	}

	fn url(&self) -> &str {
		&self.base
	}
}

impl Drop for SparseRegistry {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::SeqCst);
	}
}

fn entry(name: &str, version: &str, deps: &str, features: &str) -> String {
	format!(r#"{{"name":"{name}","vers":"{version}","cksum":"{name}-{version}-sha","deps":[{deps}],"features":{features}}}"#)
}

fn dependency(name: &str, requirement: &str, optional: bool) -> String {
	let kind = if optional { ",\"optional\":true" } else { "" };
	format!(r#"{{"name":"{name}","req":"{requirement}","features":[],"default_features":true,"kind":"normal"{kind}}}"#)
}

fn diverging_manifests() -> Vec<(&'static str, String)> {
	vec![
		(
			"/2/pa",
			format!(
				"{}\n{}\n",
				entry("pa", "1.0.0", &dependency("ch", "^1", false), "{}"),
				entry(
					"pa",
					"2.0.0",
					&format!("{},{}", dependency("ch", "^2", false), dependency("helper", "^2", true)),
					r#"{"turbo":["dep:helper"]}"#
				)
			),
		),
		(
			"/2/ch",
			format!("{}\n{}\n", entry("ch", "1.0.0", "", "{}"), entry("ch", "2.0.0", "", "{}")),
		),
		("/he/lp/helper", format!("{}\n", entry("helper", "1.0.0", "", "{}"))),
		(
			"/3/p/pin",
			format!("{}\n", entry("pin", "1.0.0", &dependency("pa", "^1", false), "{}")),
		),
	]
}

fn workspace(name: &str, registry: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-features-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"features\"\n\n[discovery]\ninclude = [\".\"]\n\n[cell.rust]\nmode = \"native\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			r#"[binary.app]
srcs = ["src/main.rs"]

[binary.app.metadata.rust.dependencies.pa]
range = {{ min = "1.0", max = "3.0" }}
features = ["turbo"]
registry = "{registry}"

[binary.app.metadata.rust.dependencies.pin]
range = {{ min = "1.0", max = "2.0" }}
registry = "{registry}"
"#
		),
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
	dir
}

fn resolve(dir: &Path) -> (bool, String) {
	run_forge(dir, &["deps", "lock"])
}

fn locked_packages(dir: &Path) -> BTreeMap<String, Vec<String>> {
	let text = std::fs::read_to_string(dir.join("forge.lock")).expect("a resolved forge.lock");
	let lock = forge_core::resolver::ForgeLock::parse(&text).expect("a parsable forge.lock");
	lock.packages
		.into_iter()
		.map(|package| (format!("{}@{}", package.name, package.version), package.dependencies))
		.collect()
}

#[test]
fn features_and_dependencies_come_from_the_selected_version_not_the_highest() {
	let registry = SparseRegistry::serve(&diverging_manifests());
	let dir = workspace("selected", registry.url());

	let (ok, log) = resolve(&dir);
	assert!(ok, "the graph has a solution and must resolve: {log}");

	let packages = locked_packages(&dir);
	assert_eq!(
		packages["pa@1.0.0"],
		vec!["ch 1.0.0"],
		"pa 1.0.0 does not declare turbo: {packages:?}"
	);
	assert_eq!(packages["ch@1.0.0"], Vec::<String>::new(), "{packages:?}");
	assert_eq!(packages["pin@1.0.0"], vec!["pa 1.0.0"], "{packages:?}");
	assert!(
		!packages.keys().any(|name| name.starts_with("helper@")),
		"turbo exists only in pa 2.0.0, so pa 1.0.0 activates no optional dependency: {packages:?}"
	);

	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_selection_is_reported_before_it_is_trusted_so_a_stale_pin_moves_rather_than_fails() {
	let registry = SparseRegistry::serve(&diverging_manifests());
	let dir = workspace("repeat", registry.url());

	let (ok, first) = resolve(&dir);
	assert!(ok, "{first}");
	let locked = std::fs::read_to_string(dir.join("forge.lock")).unwrap();

	for _ in 0..2 {
		let (ok, log) = run_forge(&dir, &["deps", "lock", "--offline"]);
		assert!(ok, "a repeat resolve over a warm store must succeed: {log}");
		assert_eq!(std::fs::read_to_string(dir.join("forge.lock")).unwrap(), locked);
	}

	let _ = std::fs::remove_dir_all(&dir);
}
