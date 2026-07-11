use crate::rhai_rt::run_forge_rhai;
use std::path::Path;

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
    "#;

	let decls = run_forge_rhai(script, Path::new(&tmp), &forge_core::Platform::host()).unwrap();
	assert_eq!(decls.len(), 4);
	assert_eq!(decls[0].name, "math");
	assert_eq!(decls[1].sources.len(), 1);
	assert_eq!(decls[2].args, vec!["--flag-a"]);
	assert_eq!(decls[3].timeout_secs, 30);

	let _ = std::fs::remove_dir_all(&tmp);
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
