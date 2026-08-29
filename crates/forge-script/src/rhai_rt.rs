use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use forge_diagnostics::ForgeDiagnostic;
use rhai::{Dynamic, EvalAltResult, Map};
use serde_json::Value as Json;

use crate::document::{FieldsBuilder, TargetDecl, TargetKind};
use crate::glob;

pub type MetadataFetcher = Rc<dyn Fn(&str) -> Result<String, String>>;

#[derive(Clone, Default)]
pub struct ResolutionContext {
	pub http_get: Option<MetadataFetcher>,
}

#[derive(Debug, Clone)]
pub struct ScriptOutput {
	pub targets: Vec<TargetDecl>,
	pub dependencies: Vec<forge_core::DependencyRequest>,
	pub requirements: Vec<forge_core::DependencyRequirement>,
	pub candidates: Vec<forge_core::PackageCandidate>,
	pub imported_lock: Option<String>,
}

pub fn run_forge_rhai(
	script: &str,
	package_dir: &Path,
	platform: &forge_core::Platform,
) -> Result<ScriptOutput, ForgeDiagnostic> {
	run_forge_rhai_configured(script, package_dir, platform, &toml::Table::new())
}

pub fn run_forge_rhai_configured(
	script: &str,
	package_dir: &Path,
	platform: &forge_core::Platform,
	config: &toml::Table,
) -> Result<ScriptOutput, ForgeDiagnostic> {
	run_forge_rhai_resolving(script, package_dir, platform, config, &ResolutionContext::default())
}

pub fn run_forge_rhai_resolving(
	script: &str,
	package_dir: &Path,
	platform: &forge_core::Platform,
	config: &toml::Table,
	resolution: &ResolutionContext,
) -> Result<ScriptOutput, ForgeDiagnostic> {
	let imported = Rc::new(RefCell::new(None::<String>));
	let decls: Rc<RefCell<Vec<TargetDecl>>> = Rc::new(RefCell::new(Vec::new()));
	let dependencies = Rc::new(RefCell::new(Vec::new()));
	let requirements = Rc::new(RefCell::new(Vec::new()));
	let candidates = Rc::new(RefCell::new(Vec::new()));
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);

	register_collectors(&mut engine, &decls, &dependencies, &requirements, &candidates);
	register_glob(&mut engine, package_dir);
	register_platform(&mut engine, platform);
	register_io(&mut engine, package_dir);
	let http_get = resolution.http_get.clone();
	engine.register_fn("http_get", move |url: &str| -> Result<String, Box<EvalAltResult>> {
		let fetch = http_get
			.as_ref()
			.ok_or("http_get is only available during explicit dependency resolution")?;
		fetch(url).map_err(Into::into)
	});

	let claim = imported.clone();
	engine.register_fn("imported_lock", move |origin: &str| -> Result<(), Box<EvalAltResult>> {
		if origin.is_empty() || claim.borrow().as_ref().is_some_and(|current| current != origin) {
			return Err("conflicting or empty imported lock authority".into());
		}
		*claim.borrow_mut() = Some(origin.to_string());
		Ok(())
	});
	let mut scope = rhai::Scope::new();
	scope.push_constant("resolving_dependencies", resolution.http_get.is_some());
	scope.push_dynamic(
		"cell_config",
		toml_value_to_dynamic(toml::Value::Table(config.clone())).map_err(|e| ForgeDiagnostic::error(101, e))?,
	);
	engine
		.eval_with_scope::<()>(&mut scope, script)
		.map_err(|e| ForgeDiagnostic::error(101, format!("rhai error: {e}")).with_source("FORGE.rhai", script))?;

	Ok(ScriptOutput {
		targets: decls.borrow().clone(),
		dependencies: dependencies.borrow().clone(),
		requirements: requirements.borrow().clone(),
		candidates: candidates.borrow().clone(),
		imported_lock: imported.borrow().clone(),
	})
}

fn register_platform(engine: &mut rhai::Engine, platform: &forge_core::Platform) {
	let os = platform.os.clone();
	engine.register_fn("platform_os", move || -> String { os.clone() });
	let arch = platform.arch.clone();
	engine.register_fn("platform_arch", move || -> String { arch.clone() });
	let abi = platform.abi.clone().unwrap_or_default();
	engine.register_fn("platform_abi", move || -> String { abi.clone() });
}

fn register_glob(engine: &mut rhai::Engine, package_dir: &Path) {
	let root = package_dir.to_path_buf();
	engine.register_fn("glob", move |pattern: &str| -> Result<rhai::Array, Box<EvalAltResult>> {
		expand(pattern, &root)
	});
	let root = package_dir.to_path_buf();
	engine.register_fn(
		"glob_files",
		move |pattern: &str| -> Result<rhai::Array, Box<EvalAltResult>> { expand(pattern, &root) },
	);
}

fn expand(pattern: &str, root: &Path) -> Result<rhai::Array, Box<EvalAltResult>> {
	let hits = glob::expand_glob(root, pattern).map_err(|e| format!("glob `{pattern}` failed: {e}"))?;
	if hits.is_empty() {
		return Err(format!("glob `{pattern}` matched nothing").into());
	}
	Ok(hits
		.into_iter()
		.map(|p: PathBuf| Dynamic::from(p.to_string_lossy().into_owned()))
		.collect())
}

fn register_io(engine: &mut rhai::Engine, package_dir: &Path) {
	let root = package_dir.to_path_buf();
	engine.register_fn("read_file", move |path: &str| -> Result<String, Box<EvalAltResult>> {
		let full = if Path::new(path).is_absolute() {
			PathBuf::from(path)
		} else {
			root.join(path)
		};
		std::fs::read_to_string(&full).map_err(|e| format!("read_file `{}`: {e}", full.display()).into())
	});
	let root = package_dir.to_path_buf();
	engine.register_fn("path_exists", move |path: &str| -> bool {
		if Path::new(path).is_absolute() {
			Path::new(path).exists()
		} else {
			root.join(path).exists()
		}
	});
	engine.register_fn("toml_decode", |text: &str| -> Result<Map, Box<EvalAltResult>> {
		toml_decode(text).map_err(Into::into)
	});
	engine.register_fn("json_decode", |text: &str| -> Result<Dynamic, Box<EvalAltResult>> {
		json_decode(text).map_err(Into::into)
	});
}

pub(crate) fn toml_decode(text: &str) -> Result<Map, String> {
	let value: toml::Value = toml::from_str(text).map_err(|e| format!("toml_decode: {e}"))?;
	let table = value.as_table().ok_or("toml top level must be a table")?;
	let mut map = Map::new();
	for (key, val) in table.clone() {
		map.insert(key.clone().into(), toml_value_to_dynamic(val)?);
	}
	Ok(map)
}

pub fn json_decode(text: &str) -> Result<Dynamic, String> {
	serde_json::from_str::<Json>(text)
		.map(json_to_dynamic)
		.map_err(|e| format!("json_decode: {e}"))
}

fn json_to_dynamic(value: serde_json::Value) -> Dynamic {
	match value {
		Json::Null => Dynamic::UNIT,
		Json::Bool(b) => Dynamic::from(b),
		Json::Number(n) => {
			if let Some(i) = n.as_i64() {
				Dynamic::from(i)
			} else {
				Dynamic::from(n.as_f64().unwrap_or_default())
			}
		}
		Json::String(s) => Dynamic::from(s),
		Json::Array(items) => Dynamic::from(items.into_iter().map(json_to_dynamic).collect::<rhai::Array>()),
		Json::Object(fields) => {
			let mut map = rhai::Map::new();
			for (k, v) in fields {
				map.insert(k.into(), json_to_dynamic(v));
			}
			Dynamic::from(map)
		}
	}
}

pub(crate) fn toml_value_to_dynamic(value: toml::Value) -> Result<Dynamic, String> {
	Ok(match value {
		toml::Value::String(s) => Dynamic::from(s),
		toml::Value::Integer(i) => Dynamic::from(i),
		toml::Value::Float(f) => Dynamic::from(f),
		toml::Value::Boolean(b) => Dynamic::from(b),
		toml::Value::Datetime(d) => Dynamic::from(d.to_string()),
		toml::Value::Array(arr) => {
			let items: Result<rhai::Array, String> = arr.into_iter().map(toml_value_to_dynamic).collect();
			Dynamic::from(items?)
		}
		toml::Value::Table(table) => {
			let mut map = Map::new();
			for (k, v) in table {
				map.insert(k.into(), toml_value_to_dynamic(v)?);
			}
			Dynamic::from(map)
		}
	})
}

fn register_collectors(
	engine: &mut rhai::Engine,
	decls: &Rc<RefCell<Vec<TargetDecl>>>,
	dependencies: &Rc<RefCell<Vec<forge_core::DependencyRequest>>>,
	requirements: &Rc<RefCell<Vec<forge_core::DependencyRequirement>>>,
	candidates: &Rc<RefCell<Vec<forge_core::PackageCandidate>>>,
) {
	macro_rules! collector {
		($fn_name:literal, $kind:expr) => {
			let sink = Rc::clone(decls);
			engine.register_fn(
				$fn_name,
				move |name: &str, fields: Map| -> Result<(), Box<EvalAltResult>> {
					let decl = build_decl($kind, name, fields).map_err(diag_to_box)?;
					sink.borrow_mut().push(decl);
					Ok(())
				},
			);
		};
	}

	collector!("library", TargetKind::Library);
	collector!("binary", TargetKind::Binary);
	collector!("test", TargetKind::Test);
	collector!("rule", TargetKind::Rule);

	let sink = Rc::clone(dependencies);
	engine.register_fn(
		"dependency",
		move |name: &str, version: &str, source: &str, checksum: &str| -> Result<(), Box<EvalAltResult>> {
			if name.is_empty() || version.is_empty() || source.is_empty() || checksum.is_empty() {
				return Err("dependency requires non-empty name, version, source, and checksum".into());
			}
			sink.borrow_mut()
				.push(forge_core::DependencyRequest::new(name, version, source, checksum));
			Ok(())
		},
	);

	let sink = Rc::clone(dependencies);
	engine.register_fn(
		"dependency",
		move |name: &str,
		      version: &str,
		      source: &str,
		      checksum: &str,
		      deps: rhai::Array|
		      -> Result<(), Box<EvalAltResult>> {
			if name.is_empty() || version.is_empty() || source.is_empty() || checksum.is_empty() {
				return Err("dependency requires non-empty name, version, source, and checksum".into());
			}
			let dependencies = deps
				.into_iter()
				.map(|v| v.into_string().map_err(|_| "dependency deps must be strings".to_string()))
				.collect::<Result<Vec<_>, _>>()?;
			sink.borrow_mut()
				.push(forge_core::DependencyRequest::new(name, version, source, checksum).with_dependencies(dependencies));
			Ok(())
		},
	);

	let sink = Rc::clone(dependencies);
	engine.register_fn(
		"dependency_git",
		move |name: &str, version: &str, url: &str, revision: &str| -> Result<(), Box<EvalAltResult>> {
			if name.is_empty() || version.is_empty() || url.is_empty() || revision.is_empty() {
				return Err("dependency_git requires non-empty name, version, url, and revision".into());
			}
			sink.borrow_mut().push(forge_core::DependencyRequest::new(
				name,
				version,
				format!("git+{url}#{revision}"),
				String::new(),
			));
			Ok(())
		},
	);

	let sink = Rc::clone(dependencies);
	engine.register_fn(
		"dependency_git",
		move |name: &str, version: &str, url: &str, revision: &str, deps: rhai::Array| -> Result<(), Box<EvalAltResult>> {
			if name.is_empty() || version.is_empty() || url.is_empty() || revision.is_empty() {
				return Err("dependency_git requires non-empty name, version, url, and revision".into());
			}
			let dependencies = deps
				.into_iter()
				.map(|v| v.into_string().map_err(|_| "dependency deps must be strings".to_string()))
				.collect::<Result<Vec<_>, _>>()?;
			sink.borrow_mut().push(
				forge_core::DependencyRequest::new(name, version, format!("git+{url}#{revision}"), String::new())
					.with_dependencies(dependencies),
			);
			Ok(())
		},
	);

	let sink = Rc::clone(requirements);
	engine.register_fn(
		"dependency_require",
		move |name: &str, minimum: &str, maximum: &str| -> Result<(), Box<EvalAltResult>> {
			let range = parse_range(minimum, maximum)?;
			sink.borrow_mut().push(forge_core::DependencyRequirement::new(name, range));
			Ok(())
		},
	);

	let sink = Rc::clone(candidates);
	engine.register_fn(
		"dependency_candidate",
		move |name: &str,
		      version: &str,
		      source: &str,
		      checksum: &str,
		      deps: rhai::Array|
		      -> Result<(), Box<EvalAltResult>> {
			if name.is_empty() || version.is_empty() || source.is_empty() || checksum.is_empty() {
				return Err("dependency_candidate requires non-empty name, version, source, and checksum".into());
			}
			let dependencies = deps.iter().map(parse_requirement).collect::<Result<Vec<_>, _>>()?;
			sink.borrow_mut().push(
				forge_core::PackageCandidate::new(name, parse_version(version)?)
					.from_source(source, checksum)
					.with_dependencies(dependencies),
			);
			Ok(())
		},
	);
}

fn parse_range(minimum: &str, maximum: &str) -> Result<forge_core::VersionRange, Box<EvalAltResult>> {
	match (minimum.is_empty(), maximum.is_empty()) {
		(true, true) => Ok(forge_core::VersionRange::full()),
		(false, true) => Ok(forge_core::VersionRange::higher_than(parse_version(minimum)?)),
		(true, false) => Ok(forge_core::VersionRange::strictly_lower_than(parse_version(maximum)?)),
		(false, false) => Ok(forge_core::VersionRange::between(
			parse_version(minimum)?,
			parse_version(maximum)?,
		)),
	}
}

fn parse_version(input: &str) -> Result<forge_core::Version, Box<EvalAltResult>> {
	forge_core::Version::parse(input).map_err(|error| -> Box<EvalAltResult> { error.into() })
}

fn parse_requirement(value: &Dynamic) -> Result<forge_core::DependencyRequirement, Box<EvalAltResult>> {
	let map = value
		.clone()
		.try_cast::<Map>()
		.ok_or_else(|| "candidate dependencies must be maps".to_string())?;
	let name = map
		.get("name")
		.and_then(|value| value.clone().into_string().ok())
		.ok_or_else(|| "candidate dependency requires string `name`".to_string())?;
	let minimum = map
		.get("min")
		.and_then(|value| value.clone().into_string().ok())
		.unwrap_or_default();
	let maximum = map
		.get("max")
		.and_then(|value| value.clone().into_string().ok())
		.unwrap_or_default();
	Ok(forge_core::DependencyRequirement::new(name, parse_range(&minimum, &maximum)?))
}

fn build_decl(kind: TargetKind, name: &str, fields: Map) -> Result<TargetDecl, ForgeDiagnostic> {
	let mut builder = FieldsBuilder::new(kind, name)?;
	for (key, value) in fields {
		apply_dynamic(&mut builder, key.to_string().as_str(), &value)?;
	}
	builder.finish()
}

fn apply_dynamic(builder: &mut FieldsBuilder, key: &str, value: &Dynamic) -> Result<(), ForgeDiagnostic> {
	fn metadata_value(value: Dynamic) -> Result<toml::Value, ForgeDiagnostic> {
		if let Some(map) = value.clone().try_cast::<Map>() {
			let table = map
				.into_iter()
				.map(|(k, v)| Ok((k.to_string(), metadata_value(v)?)))
				.collect::<Result<toml::Table, ForgeDiagnostic>>()?;
			return Ok(toml::Value::Table(table));
		}
		if let Some(list) = value.clone().try_cast::<rhai::Array>() {
			return Ok(toml::Value::Array(
				list.into_iter().map(metadata_value).collect::<Result<_, _>>()?,
			));
		}
		if let Ok(v) = value.as_bool() {
			return Ok(toml::Value::Boolean(v));
		}
		if let Ok(v) = value.as_int() {
			return Ok(toml::Value::Integer(v));
		}
		if let Ok(v) = value.as_float() {
			return Ok(toml::Value::Float(v));
		}
		if let Ok(v) = value.into_string() {
			return Ok(toml::Value::String(v));
		}
		Err(ForgeDiagnostic::error(
			103,
			"metadata values must be strings, numbers, booleans, arrays, or maps",
		))
	}

	builder.check(key)?;
	if key == "metadata" {
		if !value.is::<Map>() {
			return Err(ForgeDiagnostic::error(103, "field `metadata` expects a map"));
		}
		if let toml::Value::Table(table) = metadata_value(value.clone())? {
			return builder.metadata(table);
		}
	}
	if let Some(map) = value.clone().try_cast::<Map>() {
		if key == "env" {
			for (k, v) in map {
				let text = v
					.into_string()
					.map_err(|_| ForgeDiagnostic::error(103, format!("env `{k}` must be a string")))?;
				builder.env_entry(k.to_string(), text)?;
			}
			return Ok(());
		}
		return Err(ForgeDiagnostic::error(103, format!("field `{key}` does not accept a map")));
	}
	if let Some(list) = value.clone().try_cast::<rhai::Array>() {
		if key == "deps" {
			for item in &list {
				if let Some(dep) = item.clone().try_cast::<Map>() {
					let label = dep
						.get("target")
						.and_then(|value| value.clone().into_string().ok())
						.ok_or_else(|| ForgeDiagnostic::error(103, "dependency requires string `target`"))?;
					let edge_name = dep
						.get("edge")
						.and_then(|value| value.clone().into_string().ok())
						.unwrap_or_else(|| "hard".into());
					let edge = match edge_name.as_str() {
						"hard" | "" => forge_core::DependencyEdge::Hard,
						"order_only" => forge_core::DependencyEdge::OrderOnly,
						other => match forge_core::ConfigTransition::parse(other) {
							Some(transition) => forge_core::DependencyEdge::Transition(transition),
							None => forge_core::DependencyEdge::Tagged(other.to_string()),
						},
					};
					builder.dependency(label, edge)?;
				} else {
					builder.dependency(
						item.clone()
							.into_string()
							.map_err(|_| ForgeDiagnostic::error(103, "field `deps` expects strings or maps"))?,
						forge_core::DependencyEdge::Hard,
					)?;
				}
			}
			return Ok(());
		}
		let to_strings = |items: &[Dynamic]| -> Result<Vec<String>, ForgeDiagnostic> {
			items
				.iter()
				.map(|v| {
					v.clone()
						.into_string()
						.map_err(|_| ForgeDiagnostic::error(103, format!("field `{key}` expects strings")))
				})
				.collect()
		};
		if key == "visibility" {
			return builder.visibility_patterns(to_strings(&list)?);
		}
		let strings = to_strings(&list)?;
		if key == "outputs" {
			for s in strings {
				builder.output_entry(s)?;
			}
			return Ok(());
		}
		return builder.string_list(key, strings);
	}
	if value.is_int() {
		return builder.integer(key, value.as_int().expect("checked int"));
	}
	if value.is_bool() {
		return Err(ForgeDiagnostic::error(103, format!("field `{key}` does not take booleans")));
	}
	if let Ok(text) = value.clone().into_string() {
		if key == "outputs" {
			return builder.output_entry(text);
		}
		return builder.string(key, text);
	}
	Err(ForgeDiagnostic::error(103, format!("field `{key}` has an unsupported type")))
}

fn diag_to_box(d: ForgeDiagnostic) -> Box<EvalAltResult> {
	d.to_string().into()
}

#[test]
fn resolution_callback_is_opt_in_and_propagates_results() {
	let platform = forge_core::Platform::host();
	let config = toml::Table::new();
	let script = r#"if resolving_dependencies { throw "unexpected resolution"; }"#;
	run_forge_rhai(script, Path::new("."), &platform).unwrap();
	run_forge_rhai_configured(script, Path::new("."), &platform, &config).unwrap();
	let script = r#"http_get("https://example.invalid/metadata");"#;
	let error =
		run_forge_rhai_resolving(script, Path::new("."), &platform, &config, &ResolutionContext::default()).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("only available during explicit dependency resolution")
	);
	let calls = Rc::new(RefCell::new(Vec::new()));
	let sink = calls.clone();
	let context = ResolutionContext {
		http_get: Some(Rc::new(move |url| {
			sink.borrow_mut().push(url.to_string());
			if url == "failure" {
				Err("callback failed".into())
			} else {
				Ok("payload".into())
			}
		})),
	};
	let script = r#"if !resolving_dependencies { throw "not resolving"; } binary(http_get("metadata"), #{});"#;
	let output = run_forge_rhai_resolving(script, Path::new("."), &platform, &config, &context.clone()).unwrap();
	assert_eq!(output.targets[0].name, "payload");
	let error =
		run_forge_rhai_resolving(r#"http_get("failure");"#, Path::new("."), &platform, &config, &context).unwrap_err();
	assert!(error.to_string().contains("callback failed"));
	assert_eq!(*calls.borrow(), ["metadata", "failure"]);
	assert!(
		run_forge_rhai(
			r#"let resolving_dependencies = true; http_get("metadata");"#,
			Path::new("."),
			&platform
		)
		.is_err()
	);
}

#[test]
fn json_decode_maps_json_scalars_into_rhai() {
	let doc = json_decode(r#"{"rules": [{"logical-name": "math.core", "n": 2}], "ok": true}"#).unwrap();
	let map = doc.clone_cast::<rhai::Map>();
	assert_eq!(map.len(), 2);
	let rules = map["rules"].clone_cast::<rhai::Array>();
	assert_eq!(rules.len(), 1);
	let rule = rules[0].clone_cast::<rhai::Map>();
	assert_eq!(rule["logical-name"].to_string(), "math.core");
	assert_eq!(map["ok"].to_string(), "true");
	assert!(json_decode("not json").is_err());
}
