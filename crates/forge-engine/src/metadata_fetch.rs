use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;
use std::time::Duration;

use forge_script::rhai_rt::ResolutionContext;
use forge_script::workspace::ResolutionLimits;

pub(crate) fn resolution_context(limits: ResolutionLimits) -> ResolutionContext {
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(Duration::from_secs(limits.timeout_secs)))
		.max_redirects(0)
		.build()
		.into();
	let responses = RefCell::new(HashMap::<String, Result<String, String>>::new());
	ResolutionContext {
		http_get: Some(Rc::new(move |url| {
			let mut responses = responses.borrow_mut();
			if let Some(response) = responses.get(url) {
				return response.clone();
			}
			if responses.len() >= limits.max_requests {
				return Err(format!(
					"http_get: limit of {} distinct requests per resolution reached",
					limits.max_requests
				));
			}
			let response = fetch(&agent, url, limits.max_response_bytes);
			responses.insert(url.to_string(), response.clone());
			response
		})),
	}
}

fn fetch(agent: &ureq::Agent, url: &str, max_response_bytes: u64) -> Result<String, String> {
	let uri: ureq::http::Uri = url.parse().map_err(|_| "http_get: invalid URL".to_string())?;
	if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
		return Err("http_get: an absolute HTTP or HTTPS URL is required".into());
	}
	let mut response = agent.get(uri).call().map_err(http_error)?;
	if !response.status().is_success() {
		return Err(format!("http_get: HTTP status {}", response.status().as_u16()));
	}
	let mut bytes = Vec::new();
	response
		.body_mut()
		.as_reader()
		.take(max_response_bytes.saturating_add(1))
		.read_to_end(&mut bytes)
		.map_err(|error| http_error(error.into()))?;
	if bytes.len() as u64 > max_response_bytes {
		return Err(format!("http_get: response exceeds {max_response_bytes} byte limit"));
	}
	String::from_utf8(bytes).map_err(|_| "http_get: response is not valid UTF-8".into())
}

fn http_error(error: ureq::Error) -> String {
	match error {
		ureq::Error::StatusCode(status) => format!("http_get: HTTP status {status}"),
		ureq::Error::Timeout(_) => "http_get: request timed out".into(),
		ureq::Error::Io(error) => format!("http_get: response I/O error ({:?})", error.kind()),
		ureq::Error::HostNotFound => "http_get: host not found".into(),
		ureq::Error::TooManyRedirects => "http_get: redirects are not allowed".into(),
		_ => "http_get: HTTP request failed".into(),
	}
}

#[cfg(test)]
mod tests {
	use std::io::Write;
	use std::net::TcpListener;

	use super::*;

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

	#[test]
	fn configured_limits_control_response_size_request_budget_and_timeout() {
		let limits = ResolutionLimits {
			max_response_bytes: 4,
			timeout_secs: 1,
			max_requests: 1,
		};
		let get = resolution_context(limits.clone()).http_get.unwrap();
		let (url, server) = server(200, "", b"12345".to_vec());
		assert!(get(&url).unwrap_err().contains("4 byte limit"));
		server.join().unwrap();
		assert!(get("http://example.invalid/next").unwrap_err().contains("1 distinct"));
		let get = resolution_context(limits).http_get.unwrap();
		let (url, server) = self::server(200, "", b"1234".to_vec());
		assert_eq!(get(&url).unwrap(), "1234");
		server.join().unwrap();

		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!("http://{}", listener.local_addr().unwrap());
		let server = std::thread::spawn(move || {
			let (_stream, _) = listener.accept().unwrap();
			std::thread::sleep(Duration::from_secs(2));
		});
		let get = resolution_context(ResolutionLimits {
			timeout_secs: 1,
			..Default::default()
		})
		.http_get
		.unwrap();
		assert!(get(&url).unwrap_err().contains("timed out"));
		server.join().unwrap();
	}

	#[test]
	fn caches_successes_and_errors_and_bounds_distinct_requests() {
		let context = resolution_context(ResolutionLimits::default());
		let cloned = context.clone();
		let get = context.http_get.unwrap();
		let (url, server) = server(200, "", b"metadata".to_vec());
		assert_eq!(get(&url).unwrap(), "metadata");
		server.join().unwrap();
		assert_eq!(cloned.http_get.unwrap()(&url).unwrap(), "metadata");
		for index in 1..ResolutionLimits::default().max_requests {
			let url = format!("ftp://example.invalid/secret/{index}");
			let error = get(&url).unwrap_err();
			assert!(error.contains("HTTP or HTTPS"));
			assert!(!error.contains("secret"));
			assert_eq!(get(&url).unwrap_err(), error);
		}
		assert!(get("file:///next").unwrap_err().contains("256"));
		assert_eq!(get(&url).unwrap(), "metadata");
		assert!(
			!resolution_context(ResolutionLimits::default()).http_get.unwrap()("file:///next")
				.unwrap_err()
				.contains("256")
		);
	}

	#[test]
	fn rejects_http_errors_oversized_bodies_and_bad_utf8_without_url_secrets() {
		for (status, body, expected) in [
			(404, Vec::new(), "404"),
			(401, Vec::new(), "401"),
			(
				200,
				vec![b'x'; ResolutionLimits::default().max_response_bytes as usize + 1],
				"8388608 byte",
			),
			(200, vec![0xff], "UTF-8"),
		] {
			let (url, server) = server(status, "", body);
			let get = resolution_context(ResolutionLimits::default()).http_get.unwrap();
			let error = get(&url).unwrap_err();
			server.join().unwrap();
			assert!(error.contains(expected), "{error}");
			assert!(!error.contains("secret"), "{error}");
			assert_eq!(get(&url).unwrap_err(), error);
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
			resolution_context(ResolutionLimits::default()).http_get.unwrap()(&url)
				.unwrap_err()
				.contains("8388608 byte")
		);
		server.join().unwrap();
		let (url, server) = self::server(302, "Location: file:///secret\r\n", Vec::new());
		let error = resolution_context(ResolutionLimits::default()).http_get.unwrap()(&url).unwrap_err();
		server.join().unwrap();
		assert!(!error.contains("secret"));
	}
}
