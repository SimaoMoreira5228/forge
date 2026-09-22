use std::path::Path;

use forge_script::rhai_rt::{ResolutionContext, run_forge_rhai_configured, run_forge_rhai_resolve};

fn rust_cell() -> String {
	format!(
		"{}\n{}\n{}",
		include_str!("../../../prelude/std/rust/workspace.rhai"),
		include_str!("../../../prelude/std/rust/registry.rhai"),
		include_str!("../../../prelude/std/rust/dependencies.rhai")
	)
}

fn targets(spec: &str) -> Vec<toml::Table> {
	vec![toml::from_str(spec).unwrap()]
}

fn registry(bodies: &[(&str, &str)]) -> ResolutionContext {
	ResolutionContext {
		resolving: true,
		bytes: bodies.iter().map(|(url, body)| (url.to_string(), body.to_string())).collect(),
		..Default::default()
	}
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
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	let demo = targets(
		"[rust.dependencies.demo]\nrange = { min = \"1.0\", max = \"2.0\" }\nregistry = \"https://example.invalid/index/\"\n",
	);
	let offline = run_forge_rhai_configured(script, Path::new("."), &platform, &toml::Table::new()).unwrap();
	assert!(offline.requirements.is_empty() && offline.candidates.is_empty());
	let index = concat!(
		r#"{"vers":"1.2.3","cksum":"demo-sha","deps":[{"name":"a","req":"^0.2.3"},{"name":"bb","req":"~1.2"},{"name":"ccc","req":">=1.0, <2.0"},{"name":"ignored","req":"unsupported","kind":"dev"}]}"#,
		"\n",
		r#"{"vers":"1.3.0","cksum":"yanked","yanked":true,"deps":[]}"#,
		"\n",
		r#"{"vers":"1.4.0-rc.1","cksum":"pre","deps":[]}"#,
		"\n",
		r#"{"vers":"2.0.0","cksum":"outside","deps":[{"name":"ignored","req":"unsupported"}]}"#,
		"\n"
	);
	let resolution = registry(&[
		(
			"https://example.invalid/index/config.json",
			r#"{"dl":"https://downloads.example.invalid/crates/","auth-required":false}"#,
		),
		("https://example.invalid/index/de/mo/demo", index),
		(
			"https://example.invalid/index/1/a",
			r#"{"vers":"0.2.4","cksum":"a-sha","deps":[]}"#,
		),
		(
			"https://example.invalid/index/2/bb",
			r#"{"vers":"1.2.5","cksum":"bb-sha","deps":[]}"#,
		),
		(
			"https://example.invalid/index/3/c/ccc",
			r#"{"vers":"1.1.0","cksum":"ccc-sha","deps":[{"name":"demo","req":"^1"}]}"#,
		),
	]);
	let result = resolve_ok(script, &platform, &demo, &resolution);
	assert!(resolution.requested.borrow().is_empty());
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
    [">=1.2, <3, <2", "1.2.0", "2.0.0"],
    ["*", "", ""], ["=1.2.3", "1.2.3", "1.2.4"], [">1", "1.0.1", ""],
    ["<=2", "", "2.0.1"], ["==3.1", "3.1.0", "3.1.1"]
] {
    let range = rust_registry_requirement(sample[0]);
    if range.min != sample[1] || range.max != sample[2] { throw `wrong range: ${sample}`; }
}
for req in ["1.*", "1 || 2", "1.0-pre", "1.0+build", "1,", "01", ">=2,<1", ">="] {
    let rejected = false;
    try { rust_registry_requirement(req); } catch { rejected = true; }
    if !rejected { throw `accepted unsupported requirement: ${req}`; }
}
"#;
	run_forge_rhai_configured(&format!("{script}\n{checks}"), Path::new("."), &platform, &toml::Table::new()).unwrap();
	let invalid = registry(&[
		(
			"https://example.invalid/index/config.json",
			r#"{"dl":"https://downloads.example.invalid"}"#,
		),
		(
			"https://example.invalid/index/de/mo/demo",
			r#"{"vers":"1.2.3","cksum":"sha","deps":[{"name":"bad","req":"1.*"}]}"#,
		),
	]);
	let error = resolve_err(script, &platform, &demo, &invalid);
	assert!(error.contains("https://example.invalid/index"));
	assert!(error.contains("unsupported Rust requirement"));
}

#[test]
fn native_registry_download_templates_and_default() {
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	for (registry_decl, index, dl, expected) in [
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
			"[rust.dependencies.demo]\nrange = {{ min = '1', max = '2' }}\n{registry_decl}"
		));
		let resolution = registry(&[
			(&format!("{index}/config.json"), &serde_json::json!({ "dl": dl }).to_string()),
			(
				&format!("{index}/de/mo/demo"),
				r#"{"vers":"1.2.3","cksum":"demo-sha","deps":[]}"#,
			),
		]);
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
	let script = &rust_cell();
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
		let resolution = registry(&[("https://example.invalid/index/config.json", body)]);
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
}

#[test]
fn native_registry_declares_urls_when_the_engine_supplies_no_bytes() {
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\nregistry = 'https://example.invalid/index'");
	let resolution = ResolutionContext {
		resolving: true,
		..Default::default()
	};
	let error = resolve_err(script, &platform, &demo, &resolution);
	assert!(error.contains(forge_script::rhai_rt::UNRESOLVED_FETCH), "{error}");
	assert!(
		error.contains("Rust registry https://example.invalid/index, crate demo"),
		"{error}"
	);
	assert_eq!(*resolution.requested.borrow(), ["https://example.invalid/index/config.json"]);

	let partial = registry(&[("https://example.invalid/index/config.json", r#"{"dl":"https://cdn.invalid"}"#)]);
	let error = resolve_err(script, &platform, &demo, &partial);
	assert!(error.contains(forge_script::rhai_rt::UNRESOLVED_FETCH), "{error}");
	assert_eq!(*partial.requested.borrow(), ["https://example.invalid/index/de/mo/demo"]);

	let offline = ResolutionContext::default();
	let result = resolve_ok(script, &platform, &demo, &offline);
	assert!(offline.requested.borrow().is_empty());
	assert_eq!(result.requirements.len(), 1);
	assert!(result.candidates.is_empty());
}

#[test]
fn native_registry_activates_optional_dependencies_through_features() {
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\n");
	let resolution = |features: &str| {
		registry(&[
			("https://index.crates.io/config.json", r#"{"dl":"https://cdn.invalid"}"#),
			(
				"https://index.crates.io/de/mo/demo",
				&format!(
					r#"{{"vers":"1.2.3","cksum":"sha","features":{features},"deps":[{{"name":"helper","req":"^1","optional":true}},{{"name":"plain","req":"^1"}}]}}"#
				),
			),
			(
				"https://index.crates.io/he/lp/helper",
				r#"{"vers":"1.0.0","cksum":"h","deps":[]}"#,
			),
			(
				"https://index.crates.io/pl/ai/plain",
				r#"{"vers":"1.0.0","cksum":"p","deps":[]}"#,
			),
		])
	};
	let activated = resolve_ok(script, &platform, &demo, &resolution(r#"{"default":["dep:helper"]}"#));
	assert_eq!(resolved_names(&activated), ["demo", "helper", "plain"]);
	let inactive = resolve_ok(script, &platform, &demo, &resolution(r#"{}"#));
	assert_eq!(resolved_names(&inactive), ["demo", "plain"]);
}

#[test]
fn native_registry_honors_dependency_feature_requests() {
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\n");
	let resolution = |features: &str, default_features: bool| {
		registry(&[
			("https://index.crates.io/config.json", r#"{"dl":"https://cdn.invalid"}"#),
			(
				"https://index.crates.io/de/mo/demo",
				&format!(
					r#"{{"vers":"1.2.3","cksum":"sha","deps":[{{"name":"tool","req":"^1","features":{features},"default_features":{default_features}}}]}}"#
				),
			),
			(
				"https://index.crates.io/to/ol/tool",
				r#"{"vers":"1.0.0","cksum":"t","features":{"default":["dep:extra"]},"features2":{"extras":["dep:extra"]},"deps":[{"name":"extra","req":"^1","optional":true}]}"#,
			),
			(
				"https://index.crates.io/ex/tr/extra",
				r#"{"vers":"1.0.0","cksum":"e","deps":[]}"#,
			),
		])
	};
	let default_feature = resolve_ok(script, &platform, &demo, &resolution("[]", true));
	assert_eq!(resolved_names(&default_feature), ["demo", "extra", "tool"]);
	let requested = resolve_ok(script, &platform, &demo, &resolution(r#"["extras"]"#, false));
	assert_eq!(resolved_names(&requested), ["demo", "extra", "tool"]);
	let inactive = resolve_ok(script, &platform, &demo, &resolution("[]", false));
	assert_eq!(resolved_names(&inactive), ["demo", "tool"]);
}

#[test]
fn a_second_pass_over_one_session_parses_no_index_bytes_twice() {
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	let demo = targets(
		"[rust.dependencies.demo]\nrange = { min = \"1.0\", max = \"2.0\" }\nregistry = \"https://example.invalid/index/\"\n",
	);
	let first = registry(&[
		(
			"https://example.invalid/index/config.json",
			r#"{"dl":"https://downloads.example.invalid/crates/"}"#,
		),
		(
			"https://example.invalid/index/de/mo/demo",
			concat!(
				r#"{"vers":"1.2.3","cksum":"demo-sha","deps":[{"name":"a","req":"^0.2.3"}]}"#,
				"\n",
				r#"{"vers":"1.4.0","cksum":"next","deps":[]}"#,
				"\n"
			),
		),
		(
			"https://example.invalid/index/1/a",
			r#"{"vers":"0.2.4","cksum":"a-sha","deps":[]}"#,
		),
	]);
	let output = resolve_ok(script, &platform, &demo, &first);

	let second = ResolutionContext {
		resolving: true,
		scratch: first.scratch.clone(),
		..Default::default()
	};
	let again = resolve_ok(script, &platform, &demo, &second);

	assert_eq!(output.candidates, again.candidates);
	assert_eq!(output.requirements, again.requirements);
}

#[test]
fn native_registry_rejects_cross_registry_dependencies() {
	let script = &rust_cell();
	let platform = forge_core::Platform::host();
	let demo = targets("[rust.dependencies.demo]\nrange = {}\nregistry = 'https://example.invalid/index'");
	let resolution = registry(&[
		("https://example.invalid/index/config.json", r#"{"dl":"https://cdn.invalid"}"#),
		(
			"https://example.invalid/index/de/mo/demo",
			r#"{"vers":"1.2.3","cksum":"sha","deps":[{"name":"other","req":"^1","registry":"https://other.invalid/index"}]}"#,
		),
	]);
	let error = resolve_err(script, &platform, &demo, &resolution);
	assert!(error.contains("unsupported cross-registry dependency"), "{error}");
}
