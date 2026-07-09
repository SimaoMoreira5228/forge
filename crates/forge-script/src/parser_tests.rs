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
	assert_eq!(bin.deps, vec!["math_utils"]);
	assert_eq!(bin.env.get("OPT").map(String::as_str), Some("3"));
	assert_eq!(bin.visibility, forge_core::Visibility::Package);
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
