use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use forge_diagnostics::ForgeDiagnostic;
use rhai::{Dynamic, EvalAltResult, Map};

use crate::document::{FieldsBuilder, TargetDecl, TargetKind};
use crate::glob;

pub fn run_forge_rhai(
	script: &str,
	package_dir: &Path,
	platform: &forge_core::Platform,
) -> Result<Vec<TargetDecl>, ForgeDiagnostic> {
	let decls: Rc<RefCell<Vec<TargetDecl>>> = Rc::new(RefCell::new(Vec::new()));
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);

	register_collectors(&mut engine, &decls);
	register_glob(&mut engine, package_dir);
	register_platform(&mut engine, platform);

	engine
		.eval::<()>(script)
		.map_err(|e| ForgeDiagnostic::error(101, format!("rhai error: {e}")).with_source("FORGE.rhai", script))?;

	Ok(decls.borrow().clone())
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

fn register_collectors(engine: &mut rhai::Engine, decls: &Rc<RefCell<Vec<TargetDecl>>>) {
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
}

fn build_decl(kind: TargetKind, name: &str, fields: Map) -> Result<TargetDecl, ForgeDiagnostic> {
	let mut builder = FieldsBuilder::new(kind, name)?;
	for (key, value) in fields {
		apply_dynamic(&mut builder, key.to_string().as_str(), &value)?;
	}
	builder.finish()
}

fn apply_dynamic(builder: &mut FieldsBuilder, key: &str, value: &Dynamic) -> Result<(), ForgeDiagnostic> {
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
						"hard" => forge_core::DependencyEdge::Hard,
						"order_only" => forge_core::DependencyEdge::OrderOnly,
						"module_import" => forge_core::DependencyEdge::ModuleImport,
						"proc_macro" => forge_core::DependencyEdge::ProcMacro,
						"build_script" => forge_core::DependencyEdge::BuildScript,
						other => return Err(ForgeDiagnostic::error(103, format!("unknown dependency edge `{other}`"))),
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
