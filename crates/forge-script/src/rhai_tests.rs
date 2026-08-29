use std::path::Path;

use crate::rhai_rt::run_forge_rhai;

#[test]
fn rhai_registers_components() {
	let tmp = std::env::temp_dir().join(format!("forge-rhai-test-{}", std::process::id()));
	std::fs::create_dir_all(tmp.join("src")).unwrap();
	std::fs::write(tmp.join("src/main.cpp"), "int main(){}\n").unwrap();

	let script = r#"
        let sources = glob("src/*.cpp");

        library("math", #{
            visibility: "public",
            srcs: ["lib/math.cpp"],
            includes: ["lib"],
        });

        binary("calc", #{
            srcs: sources,
            deps: ["math"],
            compiler: "clang",
        });

        for extra in ["a", "b"] {
            test(`check_${extra}`, #{ args: [`--flag-${extra}`], timeout_secs: 30 });
        }

        dependency("demo", "1.0.0", "https://example.invalid/demo.tar.gz", "abc");
    "#;

	let output = run_forge_rhai(script, Path::new(&tmp), &forge_core::Platform::host()).unwrap();
	assert_eq!(output.targets.len(), 4);
	assert_eq!(output.targets[0].name, "math");
	assert_eq!(output.targets[1].sources.len(), 1);
	assert_eq!(output.targets[2].args, vec!["--flag-a"]);
	assert_eq!(output.targets[3].timeout_secs, 30);
	assert_eq!(output.dependencies[0].name, "demo");

	let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn rhai_metadata_matches_nested_toml() {
	let script = r#"
		binary("app", #{ metadata: #{
			literal: "${not_interpolated}",
			enabled: true,
			count: 7,
			ratio: 1.5,
			mixed: ["text", 2, false, [3]],
			empty: #{},
			options: #{ name: "value" },
			entries: [#{ name: "first" }, #{ name: "second" }],
		} });
	"#;
	let text = r#"
[binary.app.metadata]
literal = "${not_interpolated}"
enabled = true
count = 7
ratio = 1.5
mixed = ["text", 2, false, [3]]
empty = {}
[binary.app.metadata.options]
name = "value"
[[binary.app.metadata.entries]]
name = "first"
[[binary.app.metadata.entries]]
name = "second"
"#;
	let output = run_forge_rhai(script, Path::new("."), &forge_core::Platform::host()).unwrap();
	let expected = crate::parser::parse_forge_toml(text).unwrap();
	assert_eq!(output.targets[0].metadata, expected[0].metadata);
	assert!(output.targets[0].fields_set.contains("metadata"));
	assert!(output.targets[0].env.is_empty());
}

#[test]
fn rhai_metadata_defaults_and_allowed_kinds() {
	for kind in ["library", "binary", "test", "rule"] {
		let command = if kind == "rule" { "command: \"run\"," } else { "" };
		let script = format!("{kind}(\"absent\", #{{ {command} }}); {kind}(\"empty\", #{{ {command} metadata: #{{}} }});");
		let output = run_forge_rhai(&script, Path::new("."), &forge_core::Platform::host()).unwrap();
		assert!(output.targets[0].metadata.is_empty());
		assert!(!output.targets[0].fields_set.contains("metadata"));
		assert!(output.targets[1].metadata.is_empty());
		assert!(output.targets[1].fields_set.contains("metadata"));
	}
}

#[test]
fn rhai_metadata_wrong_types_and_unknown_ordinary_keys_are_rejected() {
	for value in ["\"text\"", "1", "1.5", "true", "[]", "[#{ value: 1 }]", "()"] {
		let script = format!("binary(\"app\", #{{ metadata: {value} }});");
		let err = run_forge_rhai(&script, Path::new("."), &forge_core::Platform::host()).unwrap_err();
		assert!(format!("{err}").contains("field `metadata` expects a map"));
	}
	for value in ["()", "[1, ()]", "#{ nested: () }", "'x'", "Fn(\"binary\")"] {
		let script = format!("binary(\"app\", #{{ metadata: #{{ value: {value} }} }});");
		let err = run_forge_rhai(&script, Path::new("."), &forge_core::Platform::host()).unwrap_err();
		assert!(format!("{err}").contains("metadata values must be"));
	}
	for value in ["\"text\"", "1", "true", "[]", "#{ value: \"text\" }"] {
		let script = format!("test(\"app\", #{{ metadata: #{{}}, ordinary: {value} }});");
		let err = run_forge_rhai(&script, Path::new("."), &forge_core::Platform::host()).unwrap_err();
		assert!(format!("{err}").contains("unknown key `ordinary`"));
	}
}

#[test]
fn rhai_unknown_field_errors_like_toml() {
	let err = run_forge_rhai(
		r#"binary("x", #{ src: ["a.cpp"] });"#,
		Path::new("."),
		&forge_core::Platform::host(),
	)
	.unwrap_err();
	assert!(format!("{err}").contains("unknown key"));
}

#[test]
fn rhai_collects_generic_dependency_candidates() {
	let script = r#"
        dependency_require("top", "1.0.0", "2.0.0");
        dependency_candidate(
            "top", "1.0.0", "https://example.invalid/top.tar", "top-sha",
            [#{ name: "leaf", min: "1.0.0", max: "2.0.0" }]
        );
        dependency_candidate("leaf", "1.0.0", "https://example.invalid/leaf.tar", "leaf-sha", []);
    "#;
	let output = run_forge_rhai(script, Path::new("."), &forge_core::Platform::host()).unwrap();
	assert_eq!(output.requirements.len(), 1);
	assert_eq!(output.candidates.len(), 2);
	assert_eq!(output.candidates[0].dependencies[0].name, "leaf");
}

#[test]
fn rhai_dependency_with_deps_and_toml_decode() {
	let tmp = std::env::temp_dir().join(format!("forge-rhai-deps-{}", std::process::id()));
	std::fs::create_dir_all(&tmp).unwrap();
	let lock = r#"
version = 4

[[package]]
name = "a"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123"
dependencies = ["b 1.0.0"]

[[package]]
name = "b"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "def456"
"#;
	std::fs::write(tmp.join("Cargo.lock"), lock).unwrap();
	let script = r#"
        let text = read_file("Cargo.lock");
        let lock = toml_decode(text);
        for pkg in lock["package"] {
            if pkg["checksum"] != () && pkg["checksum"] != "" {
                let deps = [];
                if pkg["dependencies"] != () {
                    for d in pkg["dependencies"] {
                        deps.push(d.split(" ")[0]);
                    }
                }
                let url = `https://crates.io/api/v1/crates/${pkg["name"]}/${pkg["version"]}/download`;
                dependency(pkg["name"], pkg["version"], url, pkg["checksum"], deps);
            }
        }
    "#;
	let output = run_forge_rhai(script, &tmp, &forge_core::Platform::host()).unwrap();
	assert_eq!(output.dependencies.len(), 2);
	assert_eq!(output.dependencies[0].name, "a");
	assert_eq!(output.dependencies[0].dependencies, vec!["b"]);
	std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn rhai_glob_miss_is_an_error() {
	let tmp = std::env::temp_dir().join(format!("forge-rhai-miss-{}", std::process::id()));
	std::fs::create_dir_all(&tmp).unwrap();
	let err = run_forge_rhai(
		r#"let x = glob("nothing/*.here");"#,
		Path::new(&tmp),
		&forge_core::Platform::host(),
	)
	.unwrap_err();
	assert!(format!("{err}").contains("matched nothing"));
	let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn overlays_replace_lists_and_merge_env() {
	use forge_core::Platform;

	let mut decls = crate::parser::parse_forge_toml(
		r#"
[binary.app]
srcs     = ["main.c"]
defines  = ["BASE"]
env      = { COMMON = "1" }

[binary.app.target."os=linux"]
defines  = ["LINUX_ONLY"]
env      = { EXTRA = "yes" }
"#,
	)
	.unwrap();
	let active_linux = Platform {
		os: "linux".into(),
		arch: "x86_64".into(),
		abi: Some("gnu".into()),
		cpu: None,
	};
	let declared = std::collections::BTreeMap::new();
	decls[0].apply_platform_overrides(&active_linux, &declared);
	assert_eq!(decls[0].defines, vec!["LINUX_ONLY"], "lists replace");
	assert_eq!(decls[0].env.get("COMMON").map(String::as_str), Some("1"), "env merges");
	assert_eq!(decls[0].env.get("EXTRA").map(String::as_str), Some("yes"));

	let mut fresh = crate::parser::parse_forge_toml(
		r#"
[binary.app]
srcs     = ["main.c"]
defines  = ["BASE"]
env      = { COMMON = "1" }

[binary.app.target."os=linux"]
defines  = ["LINUX_ONLY"]
env      = { EXTRA = "yes" }
"#,
	)
	.unwrap();
	let active_windows = Platform {
		os: "windows".into(),
		arch: "x86_64".into(),
		abi: None,
		cpu: None,
	};
	fresh[0].apply_platform_overrides(&active_windows, &declared);
	assert_eq!(fresh[0].defines, vec!["BASE"]);
	assert_eq!(fresh[0].env.get("EXTRA"), None);
	let decls = [fresh[0].clone()];
	assert_eq!(decls[0].defines, vec!["BASE"]);
	assert_eq!(decls[0].env.get("EXTRA"), None);
}

#[test]
fn named_overrides_match_by_declaration_coverage() {
	use forge_core::Platform;

	let mut decls = crate::parser::parse_forge_toml(
		r#"
[binary.fw]
srcs = ["fw.c"]

[binary.fw.target.embedded_arm]
compiler = "arm-none-eabi-gcc"
"#,
	)
	.unwrap();
	let embedded = Platform {
		os: "none".into(),
		arch: "armv7".into(),
		abi: Some("eabihf".into()),
		cpu: None,
	};
	let mut declared = std::collections::BTreeMap::new();
	declared.insert("embedded_arm".to_string(), embedded.clone());

	decls[0].apply_platform_overrides(&embedded, &declared);
	assert_eq!(decls[0].compiler.as_deref(), Some("arm-none-eabi-gcc"));

	let host = Platform::host();
	decls[0] = crate::parser::parse_forge_toml(
		"[binary.fw]\nsrcs = [\"fw.c\"]\n\n[binary.fw.target.embedded_arm]\ncompiler = \"arm-none-eabi-gcc\"\n",
	)
	.unwrap()
	.pop()
	.unwrap();
	decls[0].apply_platform_overrides(&host, &declared);
	assert_eq!(decls[0].compiler, None);
}
