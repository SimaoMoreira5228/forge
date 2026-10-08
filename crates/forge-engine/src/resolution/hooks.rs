use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;

use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::rhai_rt::{ResolutionContext, ScriptOutput, run_forge_rhai_resolve, unresolved_fetch};

use crate::resolution::transport::ResolverTransport;

type Selection = BTreeMap<String, String>;

struct Session {
	bytes: BTreeMap<String, String>,
	requested: Rc<RefCell<Vec<String>>>,
	scratch: Rc<RefCell<BTreeMap<String, String>>>,
	selected: Rc<RefCell<Option<Selection>>>,
	selections: BTreeSet<Selection>,
	conflict: String,
	outstanding: Vec<String>,
	rounds: usize,
}

pub(crate) fn run(
	script: &str,
	workspace: &Path,
	platform: &forge_core::Platform,
	cell_config: &toml::Table,
	targets: &[toml::Table],
	transport: Option<&ResolverTransport>,
	root: &str,
) -> Result<ScriptOutput, ForgeDiagnostic> {
	let Some(transport) = transport else {
		return run_forge_rhai_resolve(
			script,
			workspace,
			platform,
			cell_config,
			&ResolutionContext::default(),
			targets,
		);
	};
	let mut session = Session {
		bytes: BTreeMap::new(),
		requested: Rc::new(RefCell::new(Vec::new())),
		scratch: Rc::new(RefCell::new(BTreeMap::new())),
		selected: Rc::new(RefCell::new(None)),
		selections: BTreeSet::new(),
		conflict: String::new(),
		outstanding: Vec::new(),
		rounds: 0,
	};
	let cap = transport.max_rounds();
	loop {
		session.rounds += 1;
		if session.rounds > cap {
			return Err(ForgeDiagnostic::error(
				codes::script::PARSE_ERROR,
				format!(
					"cell resolution did not converge in {cap} rounds over [{}]",
					session.outstanding.join(", ")
				),
			));
		}
		let resolution = ResolutionContext {
			resolving: true,
			bytes: session.bytes.clone(),
			requested: session.requested.clone(),
			scratch: session.scratch.clone(),
			selected: Rc::new(RefCell::new(session.selected.borrow().clone().unwrap_or_default())),
			conflict: session.conflict.clone(),
		};
		let output = match run_forge_rhai_resolve(script, workspace, platform, cell_config, &resolution, targets) {
			Ok(output) => output,
			Err(error) => {
				if !unresolved_fetch(&error) {
					return Err(error);
				}
				session.satisfy(transport, error)?;
				continue;
			}
		};
		if output.requirements.is_empty() && output.candidates.is_empty() {
			return Ok(output);
		}
		let selection = match session.selection_of(root, &output) {
			Ok(selection) => selection,
			Err(error) => {
				if !session.conflict.is_empty() {
					return Err(error);
				}
				session.conflict = error.to_string();
				continue;
			}
		};
		if session.selected.borrow().as_ref() == Some(&selection) {
			return Ok(output);
		}
		if !session.selections.insert(selection.clone()) {
			return Err(ForgeDiagnostic::error(
				codes::script::PARSE_ERROR,
				format!(
					"cell feature closure oscillates between [{}] and [{}]",
					describe(&selection),
					describe(&session.selected.borrow().clone().unwrap_or_default()),
				),
			));
		}
		*session.selected.borrow_mut() = Some(selection);
	}
}

impl Session {
	fn satisfy(&mut self, transport: &ResolverTransport, error: ForgeDiagnostic) -> Result<(), ForgeDiagnostic> {
		let mut requested = self.requested.borrow_mut();
		requested.sort();
		requested.dedup();
		if requested.is_empty() {
			return Err(error);
		}
		self.outstanding = requested.clone();
		for url in requested.iter() {
			let text = transport.fetch(url).map_err(|reason| {
				ForgeDiagnostic::error(
					codes::script::PARSE_ERROR,
					format!("cell resolution could not fetch [{}]: {reason}", requested.join(", ")),
				)
			})?;
			self.bytes.insert(url.clone(), text);
		}
		requested.clear();
		Ok(())
	}

	fn selection_of(&self, root: &str, output: &ScriptOutput) -> Result<Selection, ForgeDiagnostic> {
		forge_core::solve(root, output.requirements.clone(), output.candidates.clone())
			.map(|resolved| {
				resolved
					.packages
					.iter()
					.map(|(name, package)| (name.clone(), package.version.to_string()))
					.collect()
			})
			.map_err(|error| ForgeDiagnostic::error(codes::script::PARSE_ERROR, error.to_string()))
	}
}

fn describe(selection: &Selection) -> String {
	selection
		.iter()
		.map(|(name, version)| format!("{name}@{version}"))
		.collect::<Vec<_>>()
		.join(", ")
}

#[cfg(test)]
mod tests {
	use std::io::{Read, Write};
	use std::net::TcpListener;
	use std::time::Duration;

	use forge_script::workspace::ResolutionLimits;

	use super::*;

	fn stub_registry(bodies: &[(&str, &str)]) -> (String, std::thread::JoinHandle<Vec<String>>) {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let bodies: Vec<(String, String)> = bodies
			.iter()
			.map(|(path, body)| (format!("{base}{path}"), body.to_string()))
			.collect();
		let authority = base.clone();
		let thread = std::thread::spawn(move || {
			let mut served = Vec::new();
			for (url, body) in bodies {
				let (mut stream, _) = listener.accept().unwrap();
				stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
				let mut request = Vec::new();
				while !request.ends_with(b"\r\n\r\n") {
					let mut byte = [0];
					stream.read_exact(&mut byte).unwrap();
					request.push(byte[0]);
				}
				let line = String::from_utf8_lossy(&request).lines().next().unwrap().to_string();
				assert_eq!(line, format!("GET {} HTTP/1.1", url.strip_prefix(&authority).unwrap()));
				served.push(line);
				write!(
					stream,
					"HTTP/1.1 200 Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
					body.len()
				)
				.unwrap();
				let _ = stream.write_all(body.as_bytes());
			}
			served
		});
		(base, thread)
	}

	fn cell(base: &str) -> String {
		format!(
			r#"
let config = json_decode(fetch(`{base}/config.json`));
let index = fetch(`{base}/index/de/mo/demo`);
dependency_candidate("demo", "1.2.3", `${{config.dl}}/demo/1.2.3/download`, "demo-sha", []);
dependency_require("demo", "1.0.0", "2.0.0");
"#
		)
	}

	fn store() -> crate::store::Store {
		store_named("shared")
	}

	fn store_named(name: &str) -> crate::store::Store {
		let root = std::env::temp_dir().join(format!("forge-hooks-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		crate::store::Store::at(&root)
	}

	#[test]
	fn the_engine_satisfies_declared_urls_and_reruns_the_hook_until_it_settles() {
		let platform = forge_core::Platform::host();
		let transport = ResolverTransport::new(ResolutionLimits::default(), store());
		let (base, server) = stub_registry(&[
			("/config.json", r#"{"dl":"https://cdn.invalid"}"#),
			("/index/de/mo/demo", r#"{"vers":"1.2.3","cksum":"demo-sha"}"#),
		]);
		let output = run(
			&cell(&base),
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap();
		let served = server.join().unwrap();
		assert_eq!(served.len(), 2, "{served:?}");
		assert!(served[0].ends_with("/config.json HTTP/1.1"), "{served:?}");
		assert!(served[1].ends_with("/index/de/mo/demo HTTP/1.1"), "{served:?}");
		assert_eq!(output.candidates.len(), 1);
		assert_eq!(
			output.candidates[0].source.as_deref(),
			Some("https://cdn.invalid/demo/1.2.3/download")
		);
		assert_eq!(output.requirements.len(), 1);
	}

	#[test]
	fn without_a_transport_the_hook_runs_once_and_declares_its_url() {
		let platform = forge_core::Platform::host();
		let script = cell("https://example.invalid");
		let error = run(&script, Path::new("."), &platform, &toml::Table::new(), &[], None, "resolve").unwrap_err();
		assert!(unresolved_fetch(&error), "{error}");
		assert!(error.to_string().contains("https://example.invalid/config.json"), "{error}");
	}

	#[test]
	fn a_second_resolution_reads_the_published_bytes_and_touches_no_network() {
		let platform = forge_core::Platform::host();
		let store = store();
		let (base, server) = stub_registry(&[
			("/config.json", r#"{"dl":"https://cdn.invalid"}"#),
			("/index/de/mo/demo", r#"{"vers":"1.2.3","cksum":"demo-sha"}"#),
		]);
		let script = cell(&base);
		let first = run(
			&script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&ResolverTransport::new(ResolutionLimits::default(), store.clone())),
			"resolve",
		)
		.unwrap();
		assert_eq!(server.join().unwrap().len(), 2);

		for _ in 0..2 {
			let output = run(
				&script,
				Path::new("."),
				&platform,
				&toml::Table::new(),
				&[],
				Some(&ResolverTransport::new(ResolutionLimits::default(), store.clone())),
				"resolve",
			)
			.unwrap();
			assert_eq!(
				output.candidates, first.candidates,
				"a cached resolution must pin the same candidates"
			);
			assert_eq!(output.requirements, first.requirements);
		}
	}

	#[test]
	fn an_offline_resolution_names_the_url_a_cold_store_does_not_hold() {
		let platform = forge_core::Platform::host();
		let store = store();
		let error = run(
			&cell("http://127.0.0.1:1"),
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&ResolverTransport::offline(ResolutionLimits::default(), store)),
			"resolve",
		)
		.unwrap_err();
		let message = error.to_string();
		assert!(message.contains("could not fetch"), "{message}");
		assert!(message.contains("http://127.0.0.1:1/config.json"), "{message}");
		assert!(message.contains("re-run without `--offline`"), "{message}");
	}

	#[test]
	fn a_network_failure_still_reports_the_transport_reason() {
		let platform = forge_core::Platform::host();
		let error = run(
			&cell("http://127.0.0.1:1"),
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&ResolverTransport::new(
				ResolutionLimits {
					timeout_secs: 1,
					..Default::default()
				},
				store(),
			)),
			"resolve",
		)
		.unwrap_err();
		let message = error.to_string();
		assert!(message.contains("could not fetch"), "{message}");
		assert!(!message.contains("--offline"), "{message}");
	}

	#[test]
	fn a_cell_error_after_a_satisfied_declaration_is_not_retried() {
		let platform = forge_core::Platform::host();
		let transport = ResolverTransport::new(ResolutionLimits::default(), store());
		let (base, server) = stub_registry(&[("/config.json", "{}")]);
		let script = format!(r#"fetch(`{base}/config.json`); throw "cell says no";"#);
		let error = run(
			&script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap_err();
		assert_eq!(server.join().unwrap().len(), 1);
		assert!(!unresolved_fetch(&error), "{error}");
		assert!(error.to_string().contains("cell says no"), "{error}");
	}

	#[test]
	fn a_transport_failure_names_the_url_the_cell_declared() {
		let platform = forge_core::Platform::host();
		let transport = ResolverTransport::new(ResolutionLimits::default(), store());
		let (base, server) = stub_registry(&[("/config.json", "{}")]);
		let script = format!(
			r#"
			fetch(`{base}/config.json`);
			fetch(`{base}/index/de/mo/demo`);
			dependency_candidate("demo", "1.2.3", "https://cdn.invalid", "demo-sha", []);
			"#
		);
		let error = run(
			&script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap_err();
		assert_eq!(server.join().unwrap().len(), 1);
		assert!(!unresolved_fetch(&error), "{error}");
		let message = error.to_string();
		assert!(message.contains("cell resolution could not fetch"), "{message}");
		assert!(message.contains(&format!("{base}/index/de/mo/demo")), "{message}");
	}

	#[test]
	fn the_engine_hands_the_cell_the_selection_it_solved_and_the_hook_settles_on_it() {
		let platform = forge_core::Platform::host();
		let transport = ResolverTransport::new(ResolutionLimits::default(), store());
		let (base, server) = stub_registry(&[
			("/config.json", r#"{"dl":"https://cdn.invalid"}"#),
			("/index/de/mo/demo", r#"{"vers":"1.2.3","cksum":"demo-sha"}"#),
		]);
		let script = format!(
			r#"
			fetch(`{base}/config.json`);
			let index = json_decode(fetch(`{base}/index/de/mo/demo`));
			if selected_packages.contains("demo") && selected_packages.demo != index.vers {{
				throw `the selection the engine handed over names a version the registry does not publish`;
			}}
			dependency_candidate("demo", index.vers, `https://cdn.invalid/demo/${{index.vers}}/download`, "demo-sha", []);
			dependency_require("demo", "1.0.0", "2.0.0");
			"#
		);
		let output = run(
			&script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap();
		assert_eq!(server.join().unwrap().len(), 2);
		assert_eq!(output.candidates.len(), 1);
		assert_eq!(output.candidates[0].version, forge_core::Version::new(1, 2, 3));
	}

	#[test]
	fn a_feature_closure_that_oscillates_fails_instead_of_spinning() {
		let platform = forge_core::Platform::host();
		let transport = ResolverTransport::new(ResolutionLimits::default(), store());
		let script = r#"
		let pin = if selected_packages.contains("demo") { selected_packages.demo } else { "1.0.0" };
		let version = if pin == "1.0.0" { "2.0.0" } else { "1.0.0" };
		dependency_candidate("demo", version, "https://cdn.invalid/demo", "demo-sha", []);
		dependency_require("demo", "0.0.1", "3.0.0");
		"#;
		let error = run(
			script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap_err();
		let message = error.to_string();
		assert!(message.contains("oscillates"), "{message}");
		assert!(message.contains("demo@1.0.0") && message.contains("demo@2.0.0"), "{message}");
	}

	#[test]
	fn a_cell_that_keeps_asking_for_new_urls_stops_at_the_round_cap() {
		let platform = forge_core::Platform::host();
		let store = store_named("cap");
		for index in 0..2 {
			let url = format!("http://127.0.0.1:1/{index}");
			std::fs::create_dir_all(store.blobs()).unwrap();
			std::fs::write(store.blob(&crate::toolchain::sync::url_digest(&url)), index.to_string()).unwrap();
		}
		let transport = ResolverTransport::new(
			ResolutionLimits {
				max_requests: 1,
				timeout_secs: 1,
				..Default::default()
			},
			store,
		);
		let script = r#"
		let round = if scratch_get("forge-cap") == () { 0 } else { parse_int(scratch_get("forge-cap")) };
		scratch_put("forge-cap", `${round + 1}`);
		fetch(`http://127.0.0.1:1/${round}`);
		dependency_require("demo", "1.0.0", "2.0.0");
		"#;
		let error = run(
			script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap_err();
		assert!(
			error
				.to_string()
				.contains("did not converge in 2 rounds over [http://127.0.0.1:1/1]"),
			"{error}"
		);
	}

	#[test]
	fn the_cell_scratch_survives_rounds_and_the_engine_never_reads_it() {
		let platform = forge_core::Platform::host();
		let transport = ResolverTransport::new(ResolutionLimits::default(), store());
		let (base, server) = stub_registry(&[("/one", "1.0.0"), ("/two", "1.5.0")]);
		let script = format!(
			r#"
			let round = if scratch_get("forge-rounds") == () {{ 0 }} else {{ parse_int(scratch_get("forge-rounds")) }};
			scratch_put("forge-rounds", `${{round + 1}}`);
			if round == 2 && scratch_get("forge-parsed") == () {{ throw "the engine emptied the cell scratch"; }}
			scratch_put("forge-parsed", "parsed");
			dependency_candidate("demo", fetch(`{base}/one`), "https://cdn.invalid/demo", "sha", []);
			dependency_candidate("other", fetch(`{base}/two`), "https://cdn.invalid/other", "sha", []);
			dependency_require("demo", "1.0.0", "2.0.0");
			dependency_require("other", "1.0.0", "2.0.0");
			"#
		);
		let output = run(
			&script,
			Path::new("."),
			&platform,
			&toml::Table::new(),
			&[],
			Some(&transport),
			"resolve",
		)
		.unwrap();
		assert_eq!(server.join().unwrap().len(), 2);
		assert_eq!(output.candidates.len(), 2);
	}
}
