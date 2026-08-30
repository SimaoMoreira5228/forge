use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use forge_script::rhai_rt::{ResolutionContext, run_forge_rhai_configured, run_forge_rhai_resolve};

fn targets(spec: &str) -> Vec<toml::Table> {
	vec![toml::from_str(spec).unwrap()]
}

fn resolve_ok(
	script: &str,
	platform: &forge_core::Platform,
	targets: &[toml::Table],
	resolution: &ResolutionContext,
) -> forge_script::rhai_rt::ScriptOutput {
	run_forge_rhai_resolve(script, Path::new("."), platform, &toml::Table::new(), resolution, targets).unwrap()
}

fn resolve_err(
	script: &str,
	platform: &forge_core::Platform,
	targets: &[toml::Table],
	resolution: &ResolutionContext,
) -> String {
	run_forge_rhai_resolve(script, Path::new("."), platform, &toml::Table::new(), resolution, targets)
		.unwrap_err()
		.to_string()
}

fn resolved_names(result: &forge_script::rhai_rt::ScriptOutput) -> Vec<String> {
	let mut names: Vec<String> = result.candidates.iter().map(|candidate| candidate.name.clone()).collect();
	names.sort();
	names
}

#[test]
fn native_sparse_resolution_and_range_subset() {
	let script = include_str!("../../../prelude/std/rust/workspace.rhai");
	let platform = forge_core::Platform::host();
	let demo = targets(
		"[rust.dependencies.demo]\nrange = { min = \"1.0\", max = \"2.0\" }\nregistry = \"https://example.invalid/index/\"\n",
	);
	let offline = run_forge_rhai_configured(script, Path::new("."), &platform, &toml::Table::new()).unwrap();
	assert!(offline.requirements.is_empty() && offline.candidates.is_empty());
	let calls = Rc::new(RefCell::new(Vec::new()));
	let observed = calls.clone();
	let resolution = ResolutionContext {
		http_get: Some(Rc::new(move |url| {
			observed.borrow_mut().push(url.to_string());
			match url {
				"https://example.invalid/index/config.json" => Ok(r#"{"dl":"https://downloads.example.invalid/crates/","auth-required":false}"#.into()),
				"https://example.invalid/index/de/mo/demo" => Ok(concat!(
					r#"{"vers":"1.2.3","cksum":"demo-sha","deps":[{"name":"a","req":"^0.2.3"},{"name":"bb","req":"~1.2"},{"name":"ccc","req":">=1.0, <2.0"},{"name":"ignored","req":"unsupported","kind":"dev"}]}"#,
					"\n",
					r#"{"vers":"1.3.0","cksum":"yanked","yanked":true,"deps":[]}"#,
					"\n",
					r#"{"vers":"1.4.0-rc.1","cksum":"pre","deps":[]}"#,
					"\n",
					r#"{"vers":"2.0.0","cksum":"outside","deps":[{"name":"ignored","req":"unsupported"}]}"#,
					"\n"
				).into()),
				"https://example.invalid/index/1/a" => Ok(r#"{"vers":"0.2.4","cksum":"a-sha","deps":[]}"#.into()),
				"https://example.invalid/index/2/bb" => Ok(r#"{"vers":"1.2.5","cksum":"bb-sha","deps":[]}"#.into()),
				"https://example.invalid/index/3/c/ccc" => Ok(r#"{"vers":"1.1.0","cksum":"ccc-sha","deps":[{"name":"demo","req":"^1"}]}"#.into()),
				_ => Err(format!("unexpected request: {url}")),
			}
		})),
	};
	let result = resolve_ok(script, &platform, &demo, &resolution);
	assert_eq!(calls.borrow().len(), 5);
	assert_eq!(calls.borrow()[0], "https://example.invalid/index/config.json");
	assert_eq!(result.candidates.len(), 4);
	for candidate in &result.candidates {
		assert_eq!(
			candidate.source.as_deref(),
			Some(
				format!(
					"https://downloads.example.invalid/crates/{}/{}/download",
					candidate.name, candidate.version
				)
				.as_str()
			)
		);
	}
	let solved = forge_core::solve("native", result.requirements, result.candidates).unwrap();
	assert_eq!(forge_core::resolver::ForgeLock::from_resolved(&solved).packages.len(), 4);
	let checks = r#"
for sample in [
    ["", "", ""], [">=1", "1.0.0", ""], ["<2", "", "2.0.0"],
    ["^1.2.3", "1.2.3", "2.0.0"], ["0.2.3", "0.2.3", "0.3.0"],
    ["^0.0.3", "0.0.3", "0.0.4"], ["^0.0", "0.0.0", "0.1.0"],
    ["~1", "1.0.0", "2.0.0"], ["~1.2.3", "1.2.3", "1.3.0"],
    [">=1.2, <3, <2", "1.2.0", "2.0.0"]
] {
    let range = rust_registry_requirement(sample[0]);
    if range.min != sample[1] || range.max != sample[2] { throw `wrong range: ${sample}`; }
}
for req in ["*", "1.*", "=1.2.3", ">1", "<=2", "1 || 2", "1.0-pre", "1.0+build", "1,", "01", ">=2,<1"] {
    let rejected = false;
    try { rust_registry_requirement(req); } catch { rejected = true; }
    if !rejected { throw `accepted unsupported requirement: ${req}`; }
}
"#;
	run_forge_rhai_configured(&format!("{script}\n{checks}"), Path::new("."), &platform, &toml::Table::new()).unwrap();
	let invalid = ResolutionContext {
		http_get: Some(Rc::new(|url| {
			if url.ends_with("/config.json") {
				Ok(r#"{"dl":"https://downloads.example.invalid"}"#.into())
			} else {
				Ok(r#"{"vers":"1.2.3","cksum":"sha","deps":[{"name":"bad","req":"*"}]}"#.into())
			}
		})),
	};
	let error = resolve_err(script, &platform, &demo, &invalid);
	assert!(error.to_string().contains("https://example.invalid/index"));
	assert!(error.to_string().contains("unsupported Rust requirement"));
}

#[test]
fn native_registry_download_templates_and_default() {
	let script = include_str!("../../../prelude/std/rust/workspace.rhai");
	let platform = forge_core::Platform::host();
	for (registry, index, dl, expected) in [
		(
			"",
			"https://index.crates.io",
			"https://static.crates.io/crates",
			"https://static.crates.io/crates/demo/1.2.3/download",
		),
		(
			"registry = 'crates.io'",
			"https://index.crates.io",
			"https://crates.io/api/v1/crates",
			"https://crates.io/api/v1/crates/demo/1.2.3/download",
		),
		(
			"registry = 'https://example.invalid/index'",
			"https://example.invalid/index",
			"http://cdn.example.invalid:8080/{prefix}/{lowerprefix}/{crate}/{version}/{sha256-checksum}/{crate}.crate",
			"http://cdn.example.invalid:8080/de/mo/de/mo/demo/1.2.3/demo-sha/demo.crate",
		),
	] {
		let demo = targets(&format!(
			"[rust.dependencies.demo]\nrange = {{ min = '1', max = '2' }}\n{registry}"
		));
		let resolution = ResolutionContext {
			http_get: Some(Rc::new(move |url| {
				if url == format!("{index}/config.json") {
					Ok(serde_json::json!({ "dl": dl }).to_string())
				} else if url == format!("{index}/de/mo/demo") {
					Ok(r#"{"vers":"1.2.3","cksum":"demo-sha","deps":[]}"#.into())
				} else {
					Err(format!("unexpected request: {url}"))
				}
			})),
		};
		let result = resolve_ok(script, &platform, &demo, &resolution);
		assert_eq!(result.candidates.len(), 1);
		assert_eq!(result.candidates[0].source.as_deref(), Some(expected));
		assert_eq!(result.candidates[0].checksum.as_deref(), Some("demo-sha"));
	}
	let checks = r#"
for sample in [["A", "1", "1"], ["AB", "2", "2"], ["AbC", "3/A", "3/a"], ["AbCdE", "Ab/Cd", "ab/cd"]] {
    let actual = rust_registry_download_url("https://cdn.invalid/{prefix}/{lowerprefix}/{crate}/{version}/{sha256-checksum}", sample[0], "1.2.3", "abc123");
    let expected = `https://cdn.invalid/${sample[1]}/${sample[2]}/${sample[0]}/1.2.3/abc123`;
    if actual != expected { throw `wrong download: ${actual}`; }
}
"#;
	run_forge_rhai_configured(&format!("{script}\n{checks}"), Path::new("."), &platform, &toml::Table::new()).unwrap();
}

#[test]
fn native_registry_rejects_invalid_config_and_urls() {
	let script = include_str!("../../../prelude/std/rust/workspace.rhai");
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\nregistry = 'https://example.invalid/index'");
	for body in [
		"not json",
		"[]",
		"null",
		"{}",
		r#"{"dl":null}"#,
		r#"{"dl":42}"#,
		r#"{"dl":""}"#,
		r#"{"dl":"https://cdn.invalid","auth-required":true}"#,
		r#"{"dl":"https://cdn.invalid","auth-required":"false"}"#,
		r#"{"dl":"https://cdn.invalid","auth-required":null}"#,
		r#"{"dl":"file:///crates"}"#,
		r#"{"dl":"/crates"}"#,
		r#"{"dl":"https://"}"#,
		r#"{"dl":"https:///crates"}"#,
		r#"{"dl":"https://bad host/crates"}"#,
		r#"{"dl":"https://cdn.invalid/\n"}"#,
		r#"{"dl":"https://cdn.invalid\\crates"}"#,
		r#"{"dl":"https://user:secret@cdn.invalid/crates"}"#,
		r#"{"dl":"https://cdn.invalid:bad/crates"}"#,
		r#"{"dl":"https://cdn.invalid:65536/crates"}"#,
		r#"{"dl":"https://cdn.invalid/{unknown}"}"#,
		r#"{"dl":"https://cdn.invalid/{crate"}"#,
		r#"{"dl":"https://cdn.invalid/crates#fragment"}"#,
		r#"{"dl":"https://cdn.invalid/crates?query"}"#,
		r#"{"dl":"https://cdn.invalid/%zz/{crate}"}"#,
	] {
		let resolution = ResolutionContext {
			http_get: Some(Rc::new(move |url| {
				assert_eq!(url, "https://example.invalid/index/config.json");
				Ok(body.into())
			})),
		};
		let error = resolve_err(script, &platform, &demo, &resolution);
		assert!(error.contains("config.json"), "{body}: {error}");
		assert!(
			error.contains("Rust registry https://example.invalid/index, crate demo"),
			"{error}"
		);
	}
	let checks = r#"
for url in ["https://", "http:///index", "https://user:pass@host/index", "https://host:bad/index", "file:///index", "https://bad host/index", "https://host/index?query", "https://host/index#fragment"] {
    let rejected = false;
    try { rust_registry_index_url(url); } catch { rejected = true; }
    if !rejected { throw `accepted invalid registry: ${url}`; }
}
"#;
	run_forge_rhai_configured(&format!("{script}\n{checks}"), Path::new("."), &platform, &toml::Table::new()).unwrap();
	let missing = ResolutionContext {
		http_get: Some(Rc::new(|url| Err(format!("not found: {url}")))),
	};
	let error = resolve_err(script, &platform, &demo, &missing);
	assert!(error.contains("config.json") && error.contains("not found"), "{error}");
}

#[test]
fn native_registry_activates_optional_dependencies_through_features() {
	let script = include_str!("../../../prelude/std/rust/workspace.rhai");
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\n");
	let resolution = |features: &str| {
		let features = features.to_string();
		ResolutionContext {
			http_get: Some(Rc::new(move |url| {
				if url.ends_with("/config.json") {
					Ok(r#"{"dl":"https://cdn.invalid"}"#.into())
				} else if url.ends_with("/de/mo/demo") {
					Ok(format!(
						r#"{{"vers":"1.2.3","cksum":"sha","features":{features},"deps":[{{"name":"helper","req":"^1","optional":true}},{{"name":"plain","req":"^1"}}]}}"#
					))
				} else if url.ends_with("/he/lp/helper") {
					Ok(r#"{"vers":"1.0.0","cksum":"h","deps":[]}"#.into())
				} else if url.ends_with("/pl/ai/plain") {
					Ok(r#"{"vers":"1.0.0","cksum":"p","deps":[]}"#.into())
				} else {
					Err(format!("unexpected request: {url}"))
				}
			})),
		}
	};
	let activated = resolve_ok(script, &platform, &demo, &resolution(r#"{"default":["dep:helper"]}"#));
	assert_eq!(resolved_names(&activated), ["demo", "helper", "plain"]);
	let inactive = resolve_ok(script, &platform, &demo, &resolution(r#"{}"#));
	assert_eq!(resolved_names(&inactive), ["demo", "plain"]);
}

#[test]
fn native_registry_honors_dependency_feature_requests() {
	let script = include_str!("../../../prelude/std/rust/workspace.rhai");
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\n");
	let resolution = |features: &str, default_features: bool| {
		let features = features.to_string();
		ResolutionContext {
			http_get: Some(Rc::new(move |url| {
				if url.ends_with("/config.json") {
					Ok(r#"{"dl":"https://cdn.invalid"}"#.into())
				} else if url.ends_with("/de/mo/demo") {
					Ok(format!(
						r#"{{"vers":"1.2.3","cksum":"sha","deps":[{{"name":"tool","req":"^1","features":{features},"default_features":{default_features}}}]}}"#
					))
				} else if url.ends_with("/to/ol/tool") {
					Ok(r#"{"vers":"1.0.0","cksum":"t","features":{"default":["dep:extra"]},"features2":{"extras":["dep:extra"]},"deps":[{"name":"extra","req":"^1","optional":true}]}"#.into())
				} else if url.ends_with("/ex/tr/extra") {
					Ok(r#"{"vers":"1.0.0","cksum":"e","deps":[]}"#.into())
				} else {
					Err(format!("unexpected request: {url}"))
				}
			})),
		}
	};
	let default_feature = resolve_ok(script, &platform, &demo, &resolution("[]", true));
	assert_eq!(resolved_names(&default_feature), ["demo", "extra", "tool"]);
	let requested = resolve_ok(script, &platform, &demo, &resolution(r#"["extras"]"#, false));
	assert_eq!(resolved_names(&requested), ["demo", "extra", "tool"]);
	let inactive = resolve_ok(script, &platform, &demo, &resolution("[]", false));
	assert_eq!(resolved_names(&inactive), ["demo", "tool"]);
}

#[test]
fn native_registry_rejects_cross_registry_dependencies() {
	let script = include_str!("../../../prelude/std/rust/workspace.rhai");
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\nregistry = 'https://example.invalid/index'");
	let resolution = ResolutionContext {
		http_get: Some(Rc::new(|url| {
			match url {
				"https://example.invalid/index/config.json" => Ok(r#"{"dl":"https://cdn.invalid"}"#.into()),
				"https://example.invalid/index/de/mo/demo" => {
					Ok(r#"{"vers":"1.2.3","cksum":"sha","deps":[{"name":"other","req":"^1","registry":"https://other.invalid/index"}]}"#.into())
				}
				_ => panic!("unexpected request: {url}"),
			}
		})),
	};
	let error = resolve_err(script, &platform, &demo, &resolution);
	assert!(error.contains("unsupported cross-registry dependency"), "{error}");
}
