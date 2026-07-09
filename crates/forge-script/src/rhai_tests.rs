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

	let decls = run_forge_rhai(script, Path::new(&tmp)).unwrap();
	assert_eq!(decls.len(), 4);
	assert_eq!(decls[0].name, "math");
	assert_eq!(decls[1].sources.len(), 1);
	assert_eq!(decls[2].args, vec!["--flag-a"]);
	assert_eq!(decls[3].timeout_secs, 30);

	let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn rhai_unknown_field_errors_like_toml() {
	let err = run_forge_rhai(r#"binary("x", #{ src: ["a.cpp"] });"#, Path::new(".")).unwrap_err();
	assert!(format!("{err}").contains("unknown key"));
}

#[test]
fn rhai_glob_miss_is_an_error() {
	let tmp = std::env::temp_dir().join(format!("forge-rhai-miss-{}", std::process::id()));
	std::fs::create_dir_all(&tmp).unwrap();
	let err = run_forge_rhai(r#"let x = glob("nothing/*.here");"#, Path::new(&tmp)).unwrap_err();
	assert!(format!("{err}").contains("matched nothing"));
	let _ = std::fs::remove_dir_all(&tmp);
}
