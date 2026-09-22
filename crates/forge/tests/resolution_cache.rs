use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

mod forge_cli;

use forge_cli::run_forge;

struct SparseRegistry {
	base: String,
	connections: Arc<AtomicUsize>,
	stop: Arc<AtomicBool>,
}

impl SparseRegistry {
	fn serve(name: &str, version: &str) -> Self {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let connections = Arc::new(AtomicUsize::new(0));
		let stop = Arc::new(AtomicBool::new(false));
		let (served, closed) = (connections.clone(), stop.clone());
		let config = format!(r#"{{"dl":"{base}/crates"}}"#).into_bytes();
		let index =
			format!(r#"{{"name":"{name}","vers":"{version}","cksum":"{name}-sha","deps":[],"features":{{}}}}"#).into_bytes();
		let sparse = format!("/de/mo/{name}").into_bytes();
		std::thread::spawn(move || {
			listener.set_nonblocking(true).unwrap();
			while !closed.load(Ordering::SeqCst) {
				let (mut stream, _) = match listener.accept() {
					Ok(accepted) => accepted,
					Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
						std::thread::sleep(Duration::from_millis(5));
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
				served.fetch_add(1, Ordering::SeqCst);
				let path = String::from_utf8_lossy(&request)
					.lines()
					.next()
					.and_then(|line| line.split_whitespace().nth(1))
					.unwrap_or("")
					.to_string();
				let (status, body) = if path == "/config.json" {
					("200 OK", &config)
				} else if path.as_bytes() == sparse.as_slice() {
					("200 OK", &index)
				} else {
					("404 Not Found", &Vec::new())
				};
				write!(
					stream,
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
					body.len()
				)
				.unwrap();
				let _ = stream.write_all(body);
			}
		});
		Self { base, connections, stop }
	}

	fn url(&self) -> &str {
		&self.base
	}

	fn connections(&self) -> usize {
		self.connections.load(Ordering::SeqCst)
	}
}

impl Drop for SparseRegistry {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::SeqCst);
	}
}

fn workspace(name: &str, registry: &str) -> std::path::PathBuf {
	let dir = std::env::temp_dir().join(format!("forge-resolve-{name}-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(dir.join("src")).unwrap();
	std::fs::write(
		dir.join("FORGE_ROOT"),
		"[project]\nname = \"resolve\"\n\n[discovery]\ninclude = [\".\"]\n\n[cell.rust]\nmode = \"native\"\n",
	)
	.unwrap();
	std::fs::write(
		dir.join("FORGE.toml"),
		format!(
			"[binary.app]\nsrcs = [\"src/main.rs\"]\n\n[binary.app.metadata.rust.dependencies.demo]\nrange = {{ min = \"1.0\", max = \"2.0\" }}\nregistry = \"{registry}\"\n"
		),
	)
	.unwrap();
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
	dir
}

fn lock_of(dir: &Path) -> String {
	std::fs::read_to_string(dir.join("forge.lock")).expect("a resolved lock")
}

#[test]
fn repeat_locking_is_byte_identical_and_free_after_the_first_run() {
	let registry = SparseRegistry::serve("demo", "1.2.3");
	let dir = workspace("repeat", registry.url());
	let (ok, log) = run_forge(&dir, &["deps", "lock"]);
	assert!(ok, "the first lock must resolve: {log}");
	let served = registry.connections();
	assert_eq!(served, 2, "a cold store fetches the registry config and the index: {log}");
	let locked = lock_of(&dir);
	assert!(locked.contains("demo") && locked.contains("1.2.3"), "{locked}");

	for args in [
		["deps", "lock"].as_slice(),
		["deps", "lock", "--offline"].as_slice(),
		["deps", "lock"].as_slice(),
	] {
		let (ok, log) = run_forge(&dir, args);
		assert!(ok, "{args:?} failed against a warm store: {log}");
		std::thread::sleep(Duration::from_millis(50));
		assert_eq!(registry.connections(), served, "{args:?} reached the network: {log}");
		assert_eq!(lock_of(&dir), locked, "{args:?} wrote a different lock");
	}

	let mirror = workspace("mirror", registry.url());
	let (ok, log) = run_forge(&mirror, &["deps", "lock"]);
	assert!(ok, "{log}");
	assert_eq!(lock_of(&mirror), locked, "a second workspace must resolve the same pins");

	let _ = std::fs::remove_dir_all(&dir);
	let _ = std::fs::remove_dir_all(&mirror);
}

#[test]
fn an_offline_lock_names_the_url_a_cold_store_does_not_hold() {
	let registry = SparseRegistry::serve("demo", "1.2.3");
	let warm = workspace("offline-warm", registry.url());
	let (ok, log) = run_forge(&warm, &["deps", "lock"]);
	assert!(ok, "{log}");

	let cold = workspace("offline-cold", registry.url());
	let (ok, log) = run_forge(&cold, &["deps", "lock", "--offline"]);
	assert!(!ok, "an offline lock over a cold store must fail: {log}");
	assert!(log.contains(&format!("{}/config.json", registry.url())), "{log}");
	assert!(log.contains("re-run without `--offline`"), "{log}");
	assert!(!cold.join("forge.lock").exists(), "{log}");

	drop(registry);
	let (ok, log) = run_forge(&cold, &["deps", "lock"]);
	assert!(!ok, "a lock over a cold store with no registry must fail: {log}");
	assert!(log.contains("could not fetch"), "{log}");
	assert!(!log.contains("re-run without"), "{log}");

	let _ = std::fs::remove_dir_all(&warm);
	let _ = std::fs::remove_dir_all(&cold);
}
