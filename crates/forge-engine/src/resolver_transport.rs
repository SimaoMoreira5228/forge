use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use forge_script::workspace::ResolutionLimits;

use crate::store::Store;
use crate::toolchain::sync::url_digest;

pub(crate) struct ResolverTransport {
	agent: ureq::Agent,
	limits: ResolutionLimits,
	store: Store,
	network: bool,
	distinct: Cell<usize>,
	responses: RefCell<HashMap<String, Result<String, String>>>,
}

impl ResolverTransport {
	pub(crate) fn new(limits: ResolutionLimits, store: Store) -> Self {
		Self::reachable(limits, store, true)
	}

	pub(crate) fn offline(limits: ResolutionLimits, store: Store) -> Self {
		Self::reachable(limits, store, false)
	}

	fn reachable(limits: ResolutionLimits, store: Store, network: bool) -> Self {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_secs(limits.timeout_secs)))
			.max_redirects(0)
			.build()
			.into();
		Self {
			agent,
			limits,
			store,
			network,
			distinct: Cell::new(0),
			responses: RefCell::new(HashMap::new()),
		}
	}

	pub(crate) fn max_rounds(&self) -> usize {
		self.limits.max_requests + 1
	}

	pub(crate) fn fetch(&self, url: &str) -> Result<String, String> {
		let mut responses = self.responses.borrow_mut();
		if let Some(response) = responses.get(url) {
			return response.clone();
		}
		if let Some(stored) = self.stored(url) {
			responses.insert(url.to_string(), Ok(stored.clone()));
			return Ok(stored);
		}
		if !self.network {
			return Err(format!(
				"offline: `{url}` is not in the global store; re-run without `--offline` to fetch it"
			));
		}
		if self.distinct.get() >= self.limits.max_requests {
			return Err(format!(
				"registry fetch: limit of {} distinct requests per resolution reached",
				self.limits.max_requests
			));
		}
		self.distinct.set(self.distinct.get() + 1);
		let response = self.request(url, self.limits.max_response_bytes).and_then(|text| {
			self.publish(url, &text)?;
			Ok(text)
		});
		responses.insert(url.to_string(), response.clone());
		response
	}

	fn stored(&self, url: &str) -> Option<String> {
		String::from_utf8(std::fs::read(self.store.blob(&url_digest(url))).ok()?).ok()
	}

	fn publish(&self, url: &str, text: &str) -> Result<(), String> {
		let staged = self.store.staging("resolve");
		std::fs::create_dir_all(self.store.blobs()).map_err(|error| store_error("stage", &self.store.blobs(), error))?;
		std::fs::write(&staged, text.as_bytes()).map_err(|error| store_error("stage", &staged, error))?;
		let blob = self.store.blob(&url_digest(url));
		let _publish = self.store.lock("store").map_err(|error| error.message)?;
		self.store.publish_file(&staged, &blob).map_err(|error| error.message)
	}

	fn request(&self, url: &str, max_response_bytes: u64) -> Result<String, String> {
		let uri: ureq::http::Uri = url.parse().map_err(|_| "registry fetch: invalid URL".to_string())?;
		if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
			return Err("registry fetch: an absolute HTTP or HTTPS URL is required".into());
		}
		let mut response = self.agent.get(uri).call().map_err(http_error)?;
		if !response.status().is_success() {
			return Err(format!("registry fetch: HTTP status {}", response.status().as_u16()));
		}
		let mut bytes = Vec::new();
		response
			.body_mut()
			.as_reader()
			.take(max_response_bytes.saturating_add(1))
			.read_to_end(&mut bytes)
			.map_err(|error| http_error(error.into()))?;
		if bytes.len() as u64 > max_response_bytes {
			return Err(format!("registry fetch: response exceeds {max_response_bytes} byte limit"));
		}
		String::from_utf8(bytes).map_err(|_| "registry fetch: response is not valid UTF-8".into())
	}
}

fn store_error(stage: &str, path: &Path, error: std::io::Error) -> String {
	format!("registry cache: {stage} `{}`: {error}", path.display())
}

fn http_error(error: ureq::Error) -> String {
	match error {
		ureq::Error::StatusCode(status) => format!("registry fetch: HTTP status {status}"),
		ureq::Error::Timeout(_) => "registry fetch: request timed out".into(),
		ureq::Error::Io(error) => format!("registry fetch: response I/O error ({:?})", error.kind()),
		ureq::Error::HostNotFound => "registry fetch: host not found".into(),
		ureq::Error::TooManyRedirects => "registry fetch: redirects are not allowed".into(),
		_ => "registry fetch: HTTP request failed".into(),
	}
}

#[cfg(test)]
mod tests {
	use std::io::Write;
	use std::net::TcpListener;

	use super::*;

	fn store_at(name: &str) -> Store {
		let root = std::env::temp_dir().join(format!("forge-resolve-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		Store::at(&root)
	}

	fn server(status: u16, headers: &str, body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!("http://{}/secret?token=secret", listener.local_addr().unwrap());
		let headers = headers.to_string();
		let thread = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().unwrap();
			stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
			stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
			let mut request = Vec::new();
			while !request.ends_with(b"\r\n\r\n") {
				let mut byte = [0];
				stream.read_exact(&mut byte).unwrap();
				request.push(byte[0]);
			}
			write!(
				stream,
				"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
				body.len()
			)
			.unwrap();
			let _ = stream.write_all(&body);
		});
		(url, thread)
	}

	fn one_shot(path: &str, body: &str) -> (String, std::thread::JoinHandle<()>) {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!("http://{}{path}", listener.local_addr().unwrap());
		let body = body.to_string();
		let thread = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().unwrap();
			stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
			let mut request = Vec::new();
			while !request.ends_with(b"\r\n\r\n") {
				let mut byte = [0];
				if stream.read_exact(&mut byte).is_err() {
					return;
				}
				request.push(byte[0]);
			}
			write!(
				stream,
				"HTTP/1.1 200 Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
				body.len()
			)
			.unwrap();
			let _ = stream.write_all(body.as_bytes());
		});
		(url, thread)
	}

	fn seed(store: &Store, url: &str, body: &[u8]) {
		std::fs::create_dir_all(store.blobs()).unwrap();
		std::fs::write(store.blob(&url_digest(url)), body).unwrap();
	}

	#[test]
	fn configured_limits_control_response_size_request_budget_and_timeout() {
		let store = store_at("limits");
		let limits = ResolutionLimits {
			max_response_bytes: 4,
			timeout_secs: 1,
			max_requests: 1,
		};
		let get = ResolverTransport::new(limits.clone(), store.clone());
		let (url, server) = server(200, "", b"12345".to_vec());
		assert!(get.fetch(&url).unwrap_err().contains("4 byte limit"));
		server.join().unwrap();
		assert!(get.fetch("http://example.invalid/next").unwrap_err().contains("1 distinct"));
		assert!(
			!store.blob(&url_digest(&url)).exists(),
			"a rejected response must not reach the store"
		);
		let get = ResolverTransport::new(limits, store);
		let (url, server) = self::server(200, "", b"1234".to_vec());
		assert_eq!(get.fetch(&url).unwrap(), "1234");
		server.join().unwrap();

		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!("http://{}", listener.local_addr().unwrap());
		let server = std::thread::spawn(move || {
			let (_stream, _) = listener.accept().unwrap();
			std::thread::sleep(Duration::from_secs(2));
		});
		let get = ResolverTransport::new(
			ResolutionLimits {
				timeout_secs: 1,
				..Default::default()
			},
			store_at("timeout"),
		);
		assert!(get.fetch(&url).unwrap_err().contains("timed out"));
		server.join().unwrap();
	}

	#[test]
	fn caches_successes_and_errors_and_bounds_distinct_requests() {
		let store = store_at("memo");
		let get = ResolverTransport::new(ResolutionLimits::default(), store.clone());
		let (url, server) = server(200, "", b"metadata".to_vec());
		assert_eq!(get.fetch(&url).unwrap(), "metadata");
		server.join().unwrap();
		assert_eq!(get.fetch(&url).unwrap(), "metadata");
		for index in 1..ResolutionLimits::default().max_requests {
			let url = format!("ftp://example.invalid/secret/{index}");
			let error = get.fetch(&url).unwrap_err();
			assert!(error.contains("HTTP or HTTPS"));
			assert!(!error.contains("secret"));
			assert_eq!(get.fetch(&url).unwrap_err(), error);
		}
		assert!(get.fetch("file:///next").unwrap_err().contains("256"));
		assert_eq!(get.fetch(&url).unwrap(), "metadata");
		assert!(
			!ResolverTransport::new(ResolutionLimits::default(), store)
				.fetch("file:///next")
				.unwrap_err()
				.contains("256")
		);
	}

	#[test]
	fn rejects_http_errors_oversized_bodies_and_bad_utf8_without_url_secrets() {
		for (index, (status, body, expected)) in [
			(404, Vec::new(), "404"),
			(401, Vec::new(), "401"),
			(
				200,
				vec![b'x'; ResolutionLimits::default().max_response_bytes as usize + 1],
				"8388608 byte",
			),
			(200, vec![0xff], "UTF-8"),
		]
		.into_iter()
		.enumerate()
		{
			let (url, server) = server(status, "", body);
			let store = store_at(&format!("reject{index}"));
			let get = ResolverTransport::new(ResolutionLimits::default(), store.clone());
			let error = get.fetch(&url).unwrap_err();
			server.join().unwrap();
			assert!(error.contains(expected), "{error}");
			assert!(!error.contains("secret"), "{error}");
			assert_eq!(get.fetch(&url).unwrap_err(), error);
			assert!(!store.blob(&url_digest(&url)).exists(), "{expected} must not be cached");
		}
	}

	#[test]
	fn bounds_decompressed_responses_and_rejects_redirects() {
		let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
		encoder
			.write_all(&vec![b'x'; ResolutionLimits::default().max_response_bytes as usize + 1])
			.unwrap();
		let (url, server) = server(200, "Content-Encoding: gzip\r\n", encoder.finish().unwrap());
		assert!(
			ResolverTransport::new(ResolutionLimits::default(), store_at("gzip"))
				.fetch(&url)
				.unwrap_err()
				.contains("8388608 byte")
		);
		server.join().unwrap();
		let (url, server) = self::server(302, "Location: file:///secret\r\n", Vec::new());
		let error = ResolverTransport::new(ResolutionLimits::default(), store_at("redirect"))
			.fetch(&url)
			.unwrap_err();
		server.join().unwrap();
		assert!(!error.contains("secret"));
	}

	#[test]
	fn a_published_response_serves_a_later_resolution_and_its_smaller_limit_never_truncates() {
		let store = store_at("publish");
		let (url, server) = one_shot("/index", "0123456789");
		assert_eq!(
			ResolverTransport::new(ResolutionLimits::default(), store.clone())
				.fetch(&url)
				.unwrap(),
			"0123456789"
		);
		server.join().unwrap();

		let narrower = ResolutionLimits {
			max_response_bytes: 4,
			timeout_secs: 1,
			max_requests: 1,
		};
		let get = ResolverTransport::new(narrower.clone(), store.clone());
		assert_eq!(get.fetch(&url).unwrap(), "0123456789", "a store hit is served whole");
		assert_eq!(get.fetch(&url).unwrap(), "0123456789", "a store hit repeats");
		assert_eq!(
			store
				.root()
				.read_dir()
				.unwrap()
				.flatten()
				.map(|entry| entry.file_name())
				.collect::<Vec<_>>(),
			vec![std::ffi::OsString::from("blobs")],
			"a published response leaves no staging path behind"
		);
		assert!(ResolverTransport::offline(narrower, store).fetch(&url).is_ok());
	}

	#[test]
	fn a_stored_response_is_never_served_for_another_url() {
		let store = store_at("collision");
		let (first, first_server) = one_shot("/first", "first body");
		let (second, second_server) = one_shot("/second", "second body");
		let get = ResolverTransport::new(ResolutionLimits::default(), store.clone());
		assert_eq!(get.fetch(&first).unwrap(), "first body");
		first_server.join().unwrap();
		assert_eq!(get.fetch(&second).unwrap(), "second body");
		second_server.join().unwrap();
		assert_ne!(url_digest(&first), url_digest(&second));
		assert_eq!(
			std::fs::read_to_string(store.blob(&url_digest(&first))).unwrap(),
			"first body"
		);
		assert_eq!(
			std::fs::read_to_string(store.blob(&url_digest(&second))).unwrap(),
			"second body"
		);
		assert!(
			ResolverTransport::new(ResolutionLimits::default(), store.clone())
				.fetch("http://127.0.0.1:1/unknown")
				.is_err(),
			"an unfetched url must never resolve to another url's bytes"
		);
	}

	#[test]
	fn a_poisoned_entry_is_replaced_instead_of_returned() {
		let store = store_at("poison");
		let (url, server) = one_shot("/index", "real body");
		seed(
			&store,
			&url,
			&[0xff, 0xfe, b'n', b'o', b't', b' ', b'u', b't', b'f', b'-', b'8'],
		);
		assert_eq!(
			ResolverTransport::new(ResolutionLimits::default(), store.clone())
				.fetch(&url)
				.unwrap(),
			"real body"
		);
		server.join().unwrap();
		assert_eq!(std::fs::read_to_string(store.blob(&url_digest(&url))).unwrap(), "real body");
	}

	#[test]
	fn offline_serves_held_bytes_and_names_a_miss() {
		let store = store_at("offline");
		let (url, server) = one_shot("/index", "held body");
		assert_eq!(
			ResolverTransport::new(ResolutionLimits::default(), store.clone())
				.fetch(&url)
				.unwrap(),
			"held body"
		);
		server.join().unwrap();
		let offline = ResolverTransport::offline(ResolutionLimits::default(), store);
		assert_eq!(offline.fetch(&url).unwrap(), "held body");
		assert_eq!(offline.fetch(&url).unwrap(), "held body");
		let error = offline.fetch("http://127.0.0.1:1/missing").unwrap_err();
		assert!(error.contains("http://127.0.0.1:1/missing"), "{error}");
		assert!(error.contains("`--offline`"), "{error}");
		assert_eq!(
			offline.fetch("http://127.0.0.1:1/missing").unwrap_err(),
			error,
			"a miss is not retried"
		);
	}

	#[test]
	fn store_hits_leave_the_request_budget_untouched() {
		let store = store_at("budget");
		let held = ResolutionLimits::default().max_requests + 1;
		for index in 0..held {
			seed(&store, &format!("http://127.0.0.1:1/held-{index}"), b"index line");
		}
		let get = ResolverTransport::new(
			ResolutionLimits {
				max_requests: 1,
				timeout_secs: 1,
				..Default::default()
			},
			store,
		);
		for index in 0..held {
			assert_eq!(get.fetch(&format!("http://127.0.0.1:1/held-{index}")).unwrap(), "index line");
		}
		let reachable = |index: &str| get.fetch(&format!("http://127.0.0.1:1/fresh-{index}")).unwrap_err();
		assert!(
			!reachable("a").contains("distinct"),
			"a store hit must not consume the request budget"
		);
		assert!(
			reachable("b").contains("1 distinct"),
			"a real fetch is the one that spends the budget"
		);
	}
}
