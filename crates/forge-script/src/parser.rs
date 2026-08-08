use forge_diagnostics::{ForgeDiagnostic, codes};
use toml_edit::{Item, Value};

use crate::document::{FieldsBuilder, PlatformOverride, TargetDecl, TargetKind};

pub fn parse_forge_toml(text: &str) -> Result<Vec<TargetDecl>, ForgeDiagnostic> {
	let doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| syntax_error(&e, text))?;

	let mut decls = Vec::new();
	for (kind_name, kind_item) in doc.as_table().iter() {
		let Some(kind) = kind_from_name(kind_name) else {
			return Err(
				ForgeDiagnostic::error(102, format!("unknown declaration table `[{kind_name}]`"))
					.with_help("expected one of: library, binary, test, rule"),
			);
		};
		let Some(targets) = kind_item.as_table() else {
			continue;
		};
		for (target_name, target_item) in targets.iter() {
			let mut builder = FieldsBuilder::new(kind, target_name)?;
			fill_fields(&mut builder, kind, target_item, text)?;
			let mut decl = builder.finish().map_err(|d| d.with_source("FORGE.toml", text))?;
			collect_overlays(&mut decl, kind, target_item, text)?;
			decls.push(decl);
		}
	}
	Ok(decls)
}

fn collect_overlays(decl: &mut TargetDecl, kind: TargetKind, item: &Item, file_text: &str) -> Result<(), ForgeDiagnostic> {
	let Some(table) = item.as_table_like() else {
		return Ok(());
	};
	for (key_name, value) in table.iter() {
		if key_name != "target" {
			continue;
		}
		let Some(matchers) = value.as_table_like() else {
			return Err(ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!("`[{}.target]` must be a table of matchers", decl.name),
			)
			.with_help(format!("example: [{}.target.\"os=linux arch=x86_64\"]", decl.name)));
		};
		if matchers.is_empty() {
			return Err(ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				format!("`[{}.target]` declares no platform matchers", decl.name),
			));
		}
		for (matcher_key, matcher_item) in matchers.iter() {
			let mut overlay_builder = FieldsBuilder::new(kind, decl.name.clone())?;
			fill_fields(&mut overlay_builder, kind, matcher_item, file_text)?;
			let overlay = overlay_builder.finish().map_err(|d| d.with_source("FORGE.toml", file_text))?;
			let is_predicates = matcher_key.contains('=');
			decl.overrides.push(PlatformOverride {
				key: matcher_key.to_string(),
				platform_name: (!is_predicates).then(|| matcher_key.to_string()),
				predicates: if is_predicates {
					matcher_key.split_whitespace().map(str::to_string).collect()
				} else {
					Vec::new()
				},
				overlay,
			});
		}
	}
	Ok(())
}

fn kind_from_name(name: &str) -> Option<TargetKind> {
	match name {
		"library" => Some(TargetKind::Library),
		"binary" => Some(TargetKind::Binary),
		"test" => Some(TargetKind::Test),
		"rule" => Some(TargetKind::Rule),
		_ => None,
	}
}

fn fill_fields(builder: &mut FieldsBuilder, kind: TargetKind, item: &Item, file_text: &str) -> Result<(), ForgeDiagnostic> {
	let Some(table) = item.as_table_like() else {
		return Ok(());
	};
	for (key_name, value) in table.iter() {
		if key_name == "target" {
			continue;
		}
		apply_field(builder, kind, key_name, value, file_text).map_err(|d| enrich(d, file_text))?;
	}
	Ok(())
}

fn apply_field(
	builder: &mut FieldsBuilder,
	_kind: TargetKind,
	key: &str,
	value: &Item,
	_file_text: &str,
) -> Result<(), ForgeDiagnostic> {
	if let Some(array) = value.as_array() {
		if key == "visibility" {
			let mut patterns = Vec::new();
			for element in array.iter() {
				patterns.push(expect_string(element, key)?);
			}
			return builder.visibility_patterns(patterns);
		}
		if !LIST_KEYS.contains(&key) {
			return builder.string_list(key, Vec::new());
		}
		if key == "deps" {
			for element in array.iter() {
				if let Some(inline) = element.as_inline_table() {
					builder.dependency(
						expect_string(
							inline
								.get("target")
								.ok_or_else(|| ForgeDiagnostic::error(103, "dependency requires `target`"))?,
							"target",
						)?,
						dependency_edge(inline.get("edge").and_then(Value::as_str)),
					)?;
				} else {
					builder.dependency(expect_string(element, key)?, forge_core::DependencyEdge::Hard)?;
				}
			}
			return Ok(());
		}
		let mut values = Vec::new();
		for element in array.iter() {
			let text = expect_string(element, key)?;
			values.push(interpolate(&text)?);
		}
		return dispatch_list(builder, key, values);
	}
	if let Some(v) = value.as_value() {
		if let Some(inline) = v.as_inline_table() {
			return apply_env(
				builder,
				inline.iter().map(|(k, val)| (k.to_string(), expect_string(val, "env"))),
			);
		}
		return apply_scalar(builder, key, v);
	}
	if let Some(table) = value.as_table_like() {
		if key == "env" {
			for (k, v) in table.iter() {
				let item_v = v
					.as_value()
					.ok_or_else(|| ForgeDiagnostic::error(103, format!("env value `{k}` must be a string")))?;
				let expanded = interpolate(&expect_string(item_v, "env")?)?;
				builder.env_entry(k.to_string(), expanded.into_string())?;
			}
			return Ok(());
		}
		Err(ForgeDiagnostic::error(103, format!("field `{key}` does not accept a table")))
	} else {
		Err(ForgeDiagnostic::error(103, format!("field `{key}` has an unsupported type")))
	}
}

fn dependency_edge(name: Option<&str>) -> forge_core::DependencyEdge {
	match name.unwrap_or("hard") {
		"hard" | "" => forge_core::DependencyEdge::Hard,
		"order_only" => forge_core::DependencyEdge::OrderOnly,
		other => match forge_core::ConfigTransition::parse(other) {
			Some(transition) => forge_core::DependencyEdge::Transition(transition),
			None => forge_core::DependencyEdge::Tagged(other.to_string()),
		},
	}
}

fn apply_scalar(builder: &mut FieldsBuilder, key: &str, value: &Value) -> Result<(), ForgeDiagnostic> {
	if let Some(text) = value.as_str() {
		let interpolated = interpolate(text)?;
		if key == "outputs" {
			return builder.output_entry(interpolated.into_string());
		}
		if LIST_KEYS.contains(&key) {
			return dispatch_list(builder, key, vec![interpolated]);
		}
		return builder.string(key, interpolated.into_string());
	}
	if let Some(n) = value.as_integer() {
		return builder.integer(key, n);
	}
	if value.is_bool() {
		return Err(ForgeDiagnostic::error(103, format!("field `{key}` does not take booleans")));
	}
	Err(ForgeDiagnostic::error(
		103,
		format!("field `{key}` expects a string, list, or integer"),
	))
}

fn apply_env(
	builder: &mut FieldsBuilder,
	entries: impl Iterator<Item = (String, Result<String, ForgeDiagnostic>)>,
) -> Result<(), ForgeDiagnostic> {
	for (k, text) in entries {
		let expanded = interpolate(&text?)?;
		builder.env_entry(k, expanded.into_string())?;
	}
	Ok(())
}

fn dispatch_list(
	builder: &mut FieldsBuilder,
	key: &str,
	values: Vec<crate::document::Value>,
) -> Result<(), ForgeDiagnostic> {
	let strings: Vec<String> = values.into_iter().map(crate::document::Value::into_string).collect();
	if key == "outputs" {
		for s in strings {
			builder.output_entry(s)?;
		}
		return Ok(());
	}
	builder.string_list(key, strings)
}

const LIST_KEYS: &[&str] = &[
	"srcs",
	"hdrs",
	"inputs",
	"outputs",
	"deps",
	"includes",
	"defines",
	"flags",
	"compatible_with",
	"system_libs",
	"link_flags",
	"data",
	"args",
];

fn expect_string(value: &Value, key: &str) -> Result<String, ForgeDiagnostic> {
	value
		.as_str()
		.map(str::to_string)
		.ok_or_else(|| ForgeDiagnostic::error(103, format!("field `{key}` expects strings")))
}

fn syntax_error(e: &toml_edit::TomlError, file_text: &str) -> ForgeDiagnostic {
	let mut d = ForgeDiagnostic::error(101, format!("syntax error: {}", e.message())).with_source("FORGE.toml", file_text);
	if let Some(span) = e.span() {
		d = d.at(span);
	}
	d
}

fn enrich(d: ForgeDiagnostic, file_text: &str) -> ForgeDiagnostic {
	d.with_source("FORGE.toml", file_text)
}

pub fn interpolate(text: &str) -> Result<crate::document::Value, ForgeDiagnostic> {
	let bytes = text.as_bytes();
	let mut out = String::new();
	let mut i = 0;
	let mut deferred: Option<crate::document::Value> = None;

	while i < bytes.len() {
		if text[i..].starts_with("\\${") {
			out.push_str("${");
			i += 3;
			continue;
		}
		if !text[i..].starts_with("${") {
			let ch = text[i..].chars().next().expect("char boundary");
			out.push(ch);
			i += ch.len_utf8();
			continue;
		}
		let Some(close_rel) = text[i + 2..].find('}') else {
			return Err(bad_expr("unclosed `${`"));
		};
		let expr = &text[i + 2..i + 2 + close_rel];
		let value = eval_expr(expr)?;
		if deferred.is_some() {
			return Err(bad_expr("only one built-in call per value is supported"));
		}
		match &value {
			crate::document::Value::Text(s) => out.push_str(s),
			_ => {
				if !out.trim().is_empty() {
					return Err(bad_expr("a built-in call must be the entire value"));
				}
				deferred = Some(value);
			}
		}
		i += close_rel + 3;
	}

	Ok(match deferred {
		Some(v) => v,
		None => crate::document::Value::Text(out),
	})
}

fn eval_expr(expr: &str) -> Result<crate::document::Value, ForgeDiagnostic> {
	let expr = expr.trim();
	let Some((name, arg)) = expr.split_once('(') else {
		return Err(bad_expr(format!("unsupported expression `{expr}`")));
	};
	let arg = arg.trim().strip_suffix(')').ok_or_else(|| bad_expr("missing closing `)`"))?;
	let arg = arg.trim().trim_matches(|c| c == '\'' || c == '"').to_string();
	match name.trim() {
		"glob" => Ok(crate::document::Value::Glob(arg)),
		"glob_files" => Ok(crate::document::Value::GlobFiles(arg)),
		other => Err(bad_expr(format!("unknown built-in `{other}`; supported: glob, glob_files"))),
	}
}

fn bad_expr(why: impl Into<String>) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::script::BAD_EXPRESSION, format!("bad expression: {}", why.into()))
		.with_help("supported form: ${glob('pattern')}")
}
