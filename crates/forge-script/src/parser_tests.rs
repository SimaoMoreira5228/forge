use crate::document::{SourceExpr, TargetKind};
use crate::parser::parse_forge_toml;

const SAMPLE: &str = r#"
[library.math_utils]
visibility = "public"
srcs       = ["lib/math.cpp", "${glob('lib/gen/*.cpp')}"]
includes   = ["lib"]
defines    = ["MATH_STATIC"]
compatible_with = ["os=linux"]

[binary.calc]
deps      = ["math_utils"]
compiler  = "clang"
standard  = "c++20"
env       = { OPT = "3" }
"#;

#[test]
fn parses_declarations_with_fields() {
	let decls = parse_forge_toml(SAMPLE).unwrap();
	assert_eq!(decls.len(), 2);

	let lib = &decls[0];
	assert_eq!(lib.kind, TargetKind::Library);
	assert_eq!(lib.name, "math_utils");
	assert_eq!(lib.visibility, forge_core::Visibility::Public);
	assert_eq!(lib.sources.len(), 2);
	assert_eq!(lib.sources[0], SourceExpr::File("lib/math.cpp".into()));
	assert_eq!(lib.sources[1], SourceExpr::Glob("lib/gen/*.cpp".into()));
	assert_eq!(lib.compatible_with, vec!["os=linux"]);
}

#[test]
fn binary_inherits_defaults_and_env_table() {
	let decls = parse_forge_toml(SAMPLE).unwrap();
	let bin = &decls[1];
	assert_eq!(bin.kind, TargetKind::Binary);
	assert_eq!(bin.compiler.as_deref(), Some("clang"));
	assert_eq!(bin.deps[0].label, "math_utils");
	assert_eq!(bin.deps[0].edge, forge_core::DependencyEdge::Hard);
	assert_eq!(bin.env.get("OPT").map(String::as_str), Some("3"));
	assert_eq!(bin.visibility, forge_core::Visibility::Package);
}

#[test]
fn parses_typed_dependency_edges() {
	let decls = parse_forge_toml(
		r#"[binary.app]
deps = [
  "//lib:base",
  { target = "//tools:macro", edge = "proc_macro" },
  { target = "//tools:gen", edge = "build_script" },
]
"#,
	)
	.unwrap();
	let deps = &decls[0].deps;
	assert_eq!(deps[1].edge, forge_core::DependencyEdge::Tagged("proc_macro".into()));
	assert_eq!(deps[2].edge, forge_core::DependencyEdge::Tagged("build_script".into()));
}

#[test]
fn unknown_key_is_rejected_with_suggestion() {
	let text = "[binary.app]\nsrc = \"a.c\"\n";
	let err = parse_forge_toml(text).unwrap_err();
	assert!(format!("{err}").contains("unknown key `src`"));
	assert!(format!("{err}").contains("did you mean `srcs`?"));
}

#[test]
fn unknown_declaration_table_rejected() {
	let err = parse_forge_toml("[widget.x]\n").unwrap_err();
	assert!(format!("{err}").contains("unknown declaration table"));
}

#[test]
fn glob_interpolation_and_escapes() {
	let decls = parse_forge_toml(
		"[rule.gen]\ncommand = \"gen\"\ninputs = [\"${glob('schemas/*.proto')}\"]\noutputs = [\"generated/\"]\n",
	)
	.unwrap();
	let r = &decls[0];
	assert_eq!(r.inputs[0], SourceExpr::Glob("schemas/*.proto".into()));
	assert_eq!(r.outputs, vec![("generated".into(), true)]);
}

#[test]
fn bad_expression_reports_helpfully() {
	let err = parse_forge_toml("[binary.a]\nsrcs = \"${frobnicate('x')}\"\n").unwrap_err();
	assert!(format!("{err}").contains("unknown built-in `frobnicate`"));
}

#[test]
fn rule_requires_command() {
	let err = parse_forge_toml("[rule.bad]\ninputs = [\"x\"]\n").unwrap_err();
	assert!(format!("{err}").contains("requires `command`"));
}

#[test]
fn target_tables_become_overlays() {
	let decls = parse_forge_toml(
		r#"
[binary.app]
srcs = ["src/main.c"]
flags = ["-Wall"]

[binary.app.target."os=linux arch=x86_64"]
link_flags = ["-fuse-ld=mold"]

[binary.app.target."os=windows"]
flags = ["/W4"]
"#,
	)
	.unwrap();
	let bin = &decls[0];
	assert_eq!(bin.overrides.len(), 2);
	assert_eq!(bin.overrides[0].predicates, vec!["os=linux", "arch=x86_64"]);
	assert_eq!(bin.overrides[1].predicates, vec!["os=windows"]);
	assert_eq!(bin.overrides[0].link_flags_overlay_len(), 1);
	assert!(bin.overrides[0].platform_name.is_none());
}

#[test]
fn named_platform_matchers_are_recognized() {
	let decls = parse_forge_toml(
		r#"
[binary.fw]
srcs = ["fw.c"]

[binary.fw.target.embedded_arm]
linker = "arm-none-eabi-ld"
"#,
	)
	.unwrap();
	let fw = &decls[0];
	assert_eq!(fw.overrides.len(), 1);
	assert_eq!(fw.overrides[0].platform_name.as_deref(), Some("embedded_arm"));
}

#[test]
fn unknown_field_inside_target_overlay_is_rejected() {
	let err =
		parse_forge_toml("[binary.a]\nsrcs=[\"a.c\"]\n\n[binary.a.target.\"os=linux\"]\nsrcc = [\"x\"]\n").unwrap_err();
	assert!(format!("{err}").contains("unknown key"));
}

#[test]
fn metadata_preserves_nested_and_inline_values() {
	let nested = r#"
[binary.app.metadata]
literal = "${not_interpolated}"
enabled = true
count = 7
ratio = 1.5
created = 2026-09-17T12:00:00Z
mixed = ["text", 2, false, [3]]
empty = {}
[binary.app.metadata.options]
name = "value"
[[binary.app.metadata.entries]]
name = "first"
[[binary.app.metadata.entries]]
name = "second"
"#;
	let inline = r#"
[binary.app]
metadata = { literal = "${not_interpolated}", enabled = true, count = 7, ratio = 1.5, created = 2026-09-17T12:00:00Z, mixed = ["text", 2, false, [3]], empty = {}, options = { name = "value" }, entries = [{ name = "first" }, { name = "second" }] }
"#;
	let nested = parse_forge_toml(nested).unwrap().remove(0);
	let inline = parse_forge_toml(inline).unwrap().remove(0);
	assert_eq!(nested.metadata, inline.metadata);
	assert!(nested.fields_set.contains("metadata"));
	assert!(inline.fields_set.contains("metadata"));
	assert_eq!(nested.metadata["literal"].as_str(), Some("${not_interpolated}"));
	assert_eq!(nested.metadata["enabled"].as_bool(), Some(true));
	assert_eq!(nested.metadata["count"].as_integer(), Some(7));
	assert_eq!(nested.metadata["ratio"].as_float(), Some(1.5));
	assert!(nested.metadata["created"].is_datetime());
	assert_eq!(nested.metadata["options"]["name"].as_str(), Some("value"));
	assert_eq!(nested.metadata["entries"][1]["name"].as_str(), Some("second"));
	assert!(nested.env.is_empty());
}

#[test]
fn metadata_defaults_and_allowed_kinds() {
	for kind in ["library", "binary", "test", "rule"] {
		let command = if kind == "rule" { "command = \"run\"\n" } else { "" };
		let text = format!("[{kind}.app]\n{command}");
		let decl = parse_forge_toml(&text).unwrap().remove(0);
		assert!(decl.metadata.is_empty());
		assert!(!decl.fields_set.contains("metadata"));
		let decl = parse_forge_toml(&format!("{text}metadata = {{}}\n")).unwrap().remove(0);
		assert!(decl.metadata.is_empty());
		assert!(decl.fields_set.contains("metadata"));
	}
}

#[test]
fn metadata_wrong_types_and_unknown_ordinary_keys_are_rejected() {
	for value in ["\"text\"", "1", "1.5", "true", "[]", "[{ value = 1 }]"] {
		let err = parse_forge_toml(&format!("[binary.app]\nmetadata = {value}\n")).unwrap_err();
		assert!(format!("{err}").contains("field `metadata` expects a table"));
	}
	let err = parse_forge_toml("[[binary.app.metadata]]\nvalue = 1\n").unwrap_err();
	assert!(format!("{err}").contains("field `metadata` expects a table"));
	for value in ["\"text\"", "1", "true", "[]", "{ value = \"text\" }"] {
		let err = parse_forge_toml(&format!("[test.app]\nmetadata = {{}}\nordinary = {value}\n")).unwrap_err();
		assert!(format!("{err}").contains("unknown key `ordinary`"));
	}
}

#[test]
fn metadata_overlays_replace_instead_of_merge() {
	let active = forge_core::Platform {
		os: "linux".into(),
		arch: "x86_64".into(),
		abi: None,
		cpu: None,
	};
	for (overlay, expected) in [
		("metadata = { nested = { new = true } }", "nested = { new = true }"),
		("metadata = {}", ""),
		("flags = []", "base = true\nnested = { old = true }"),
	] {
		let text = format!(
			"[binary.app]\nmetadata = {{ base = true, nested = {{ old = true }} }}\n[binary.app.target.\"os=linux\"]\n{overlay}\n"
		);
		let mut decl = parse_forge_toml(&text).unwrap().remove(0);
		decl.apply_platform_overrides(&active, &Default::default());
		assert_eq!(decl.metadata, toml::from_str::<toml::Table>(expected).unwrap());
	}
}

#[test]
fn empty_target_table_is_rejected() {
	let err = parse_forge_toml("[binary.a]\nsrcs=[\"a.c\"]\n\n[binary.a.target]\n").unwrap_err();
	assert!(format!("{err}").contains("no platform matchers"));
}
