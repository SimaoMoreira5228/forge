use super::*;

#[test]
fn rust_cell_owns_target_predicates() {
	let cell = crate::std_cells::cell_script("rust").unwrap();
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	register_memo(&mut engine);
	let checks = r#"
let ctx = #{ platform_os: "linux", platform_arch: "x86_64", platform_abi: "gnu", profile: #{ is_debug: true } };
for sample in [
    ["cfg(windows)", false], ["cfg(unix)", true], ["cfg(debug_assertions)", true],
    ["cfg(target_os = \"linux\")", true], ["cfg(target_os = \"lin ux\")", false],
    ["cfg(target_arch = \"aarch64\")", false], ["cfg(target_pointer_width = \"64\")", true],
    ["cfg(all(any(target_os = \"linux\", target_os = \"android\"), not(any(all(target_os = \"linux\", target_env = \"\"), getrandom_backend = \"custom\"))))", true],
    ["cfg(all())", true], ["cfg(any())", false], ["cfg(all(unix,))", true],
    ["x86_64-unknown-linux-gnu", true], ["x86_64-pc-windows-msvc", false]
] {
    if rust_cfg_matches(ctx, sample[0]) != sample[1] { throw `incorrect cfg result: ${sample}`; }
}
for expression in ["cfg(not())", "cfg(not(unix, windows))", "cfg(all(unix)", "cfg(unix))", "cfg(all(,unix))", "cfg(target_os = linux)", "cfg(target_os = \"linux)", "cfg(unix windows)", "cfg(foo(unix))"] {
    let rejected = false;
    try { rust_cfg_matches(ctx, expression); } catch { rejected = true; }
    if !rejected { throw `accepted invalid cfg: ${expression}`; }
}
ctx.platform_os = "darwin";
if !rust_cfg_matches(ctx, "cfg(all(target_os = \"macos\", target_vendor = \"apple\"))") { throw "Darwin mapping failed"; }
ctx.platform_os = "windows";
if !rust_cfg_matches(ctx, "cfg(all(windows, not(unix), target_family = \"windows\"))") { throw "Windows mapping failed"; }
ctx.platform_os = "linux";
let manifest = #{ target: #{ "cfg(windows)": #{ dependencies: #{ win: "1" } }, "cfg(unix)": #{ dependencies: #{ posix: "1" } } } };
if active_dependency_names(ctx, manifest) != ["posix"] { throw "target dependency filtering failed"; }
let build_manifest = #{ dependencies: #{ posix: "1" }, "build-dependencies": #{ pkg_config: "1" } };
if active_build_names(ctx, build_manifest) != ["pkg_config"] { throw "build dependency filtering failed"; }
"#;
	engine.eval::<()>(&format!("{cell}\n{checks}")).unwrap();
}

#[test]
fn rust_profile_args_translate_optimization_settings() {
	let cell = crate::std_cells::cell_script("rust").unwrap();
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	register_memo(&mut engine);
	let checks = r#"
let empty = #{};
let debug_profile = #{ opt_level: "0", "debug": "full", lto: "off", strip: "none", options: empty };
if rust_crate_profile_args(debug_profile, true) != ["-C", "opt-level=0", "-C", "debuginfo=2"] { throw "wrong debug args"; }
if rust_profile_args(#{ profile: debug_profile }) != ["-C", "opt-level=0", "-C", "debuginfo=2"] { throw "wrong ctx debug args"; }
let release = #{ opt_level: "3", "debug": "none", lto: "fat", strip: "symbols", options: empty };
if rust_crate_profile_args(release, true) != ["-C", "opt-level=3", "-C", "debuginfo=0", "-C", "lto", "-C", "strip=symbols"] { throw "wrong release args"; }
if rust_crate_profile_args(release, false) != ["-C", "opt-level=3", "-C", "debuginfo=0"] { throw "wrong host release args"; }
let size = #{ opt_level: "z", "debug": "line-tables-only", lto: "thin", strip: "debuginfo", options: empty };
if rust_crate_profile_args(size, true) != ["-C", "opt-level=z", "-C", "debuginfo=line-tables-only", "-C", "lto=thin", "-C", "strip=debuginfo"] { throw "wrong size args"; }
let host = #{ opt_level: "0", "debug": "limited", lto: "off", strip: "none", options: empty };
let with_build = #{ opt_level: "3", "debug": "none", lto: "off", strip: "none", options: empty, build: host };
if rust_host_profile(#{ profile: with_build }) != host { throw "build override not selected"; }
if rust_host_profile(#{ profile: release }) != release { throw "main profile not used without override"; }
let options = #{ "codegen-units": 1, "panic": "abort", "overflow-checks": false, "split-debuginfo": "packed", "rustflags": ["-C", "target-cpu=native"] };
let option_args = rust_crate_profile_args(#{ opt_level: "3", "debug": "none", lto: "off", strip: "none", options: options }, true);
let expected = ["-C", "opt-level=3", "-C", "debuginfo=0", "-C", "codegen-units=1", "-C", "panic=abort", "-C", "overflow-checks=off", "-C", "split-debuginfo=packed", "-C", "target-cpu=native"];
if option_args != expected { throw `wrong option args: ${option_args}`; }
"#;
	engine.eval::<()>(&format!("{cell}\n{checks}")).unwrap();
}

#[test]
fn rust_cell_resolves_build_and_linked_dependencies() {
	let cell = crate::std_cells::cell_script("rust").unwrap();
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	engine.set_max_call_levels(128);
	register_memo(&mut engine);
	register_graph_ops(&mut engine);
	register_plan_ops(&mut engine);
	engine.register_fn("plan_of", |value: Map| CellPlan::new(Dynamic::from(value)));
	let checks = r#"
let ctx = #{ platform_os: "linux", platform_arch: "x86_64", platform_abi: "gnu", profile: #{ name: "debug", is_debug: true } };
let manifest = #{
    "build-dependencies": #{ cc: "1" },
    "target": #{ "cfg(unix)": #{ "build-dependencies": #{ "pkg-config": "1" } } },
};
let sections = active_build_sections(ctx, manifest);
if sections.len() != 2 { throw `expected 2 build sections, got ${sections.len()}`; }
let plan = plan_of(#{
    "by_name": #{ "aws-lc-sys": "aws-lc-sys@0.45.0", zeroize: "zeroize@1.8.1" },
    "units": #{
        "aws-lc-sys@0.45.0": #{ key: "aws-lc-sys@0.45.0", name: "aws-lc-sys", version: "0.45.0", links: "aws_lc_0_45_0" },
        "zeroize@1.8.1": #{ key: "zeroize@1.8.1", name: "zeroize", version: "1.8.1", links: () },
    },
    "artifacts": #{
        "aws-lc-sys@0.45.0": #{ host: "forge-out/lib/deps/host/debug/libaws_lc_sys.rlib" },
        "zeroize@1.8.1": #{ target: "forge-out/lib/deps/debug/libzeroize.rlib" },
    },
    "graph": #{
        "aws-lc-sys@0.45.0": #{ full: [], host: ["aws-lc-sys@0.45.0"] },
        "zeroize@1.8.1": #{ full: [], host: [] },
    },
    "build_sections": #{
        "aws-lc-sys@0.45.0": [#{ "aws-lc-sys": "1" }],
        "zeroize@1.8.1": [],
    },
    "dep_sections": #{
        "aws-lc-sys@0.45.0": [#{ "aws-lc-sys": #{ optional: true }, zeroize: "1" }],
        "zeroize@1.8.1": [],
    },
});
let externs = build_dependency_externs(ctx, plan, (), plan.get("build_sections", "aws-lc-sys@0.45.0"));
if externs.len() != 1 { throw `expected 1 build extern, got ${externs.len()}`; }
if externs[0].path != "forge-out/lib/deps/host/debug/libaws_lc_sys.rlib" { throw `build dependency must link a host unit: ${externs[0].path}`; }
if externs[0].closure != ["forge-out/lib/deps/host/debug/libaws_lc_sys.rlib"] { throw `the build script sandbox must carry the build dependency: ${externs[0].closure}`; }
let linked = linked_dependencies(ctx, plan, (), plan.get("dep_sections", "aws-lc-sys@0.45.0"));
if linked.len() != 1 { throw `expected 1 linked dependency, got ${linked.len()}`; }
if linked[0].links != "aws_lc_0_45_0" { throw `wrong links: ${linked[0].links}`; }
if linked[0].dir != "forge-out/build/debug/aws-lc-sys-0.45.0" { throw `wrong dir: ${linked[0].dir}`; }
"#;
	let mut scope = rhai::Scope::new();
	engine
		.eval_with_scope::<()>(&mut scope, &format!("{cell}\n{checks}"))
		.unwrap();
}

#[test]
fn rust_cell_builds_dependencies_for_host_and_target() {
	let cell = crate::std_cells::cell_script("rust").unwrap();
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	engine.set_max_call_levels(128);
	register_memo(&mut engine);
	register_graph_ops(&mut engine);
	let checks = r#"
let ctx = #{
    name: "app",
    metadata: #{ rust: #{ dependencies: #{ helper: "1.0.0", dual: "1.0.0" }, "build-dependencies": #{ cc: "1.0.0", dual: "1.0.0" } } },
};
let meta = #{
    "helper@1.0.0": #{ src: #{ name: "helper", version: "1.0.0" }, manifest: #{}, proc_macro: false },
    "dual@1.0.0": #{ src: #{ name: "dual", version: "1.0.0" }, manifest: #{}, proc_macro: false },
    "shared@1.0.0": #{ src: #{ name: "shared", version: "1.0.0" }, manifest: #{}, proc_macro: false },
    "cc@1.0.0": #{ src: #{ name: "cc", version: "1.0.0" }, manifest: #{}, proc_macro: false },
    "shlex@1.0.0": #{ src: #{ name: "shlex", version: "1.0.0" }, manifest: #{}, proc_macro: false },
    "deep@1.0.0": #{ src: #{ name: "deep", version: "1.0.0" }, manifest: #{}, proc_macro: false },
    "codegen@1.0.0": #{ src: #{ name: "codegen", version: "1.0.0" }, manifest: #{ lib: #{ "proc-macro": true } }, proc_macro: true },
    "derive@1.0.0": #{ src: #{ name: "derive", version: "1.0.0" }, manifest: #{ lib: #{ "proc-macro": true } }, proc_macro: true },
    "spare@1.0.0": #{ src: #{ name: "spare", version: "1.0.0" }, manifest: #{}, proc_macro: false },
};
let by_name = #{
    "helper": "helper@1.0.0", "dual": "dual@1.0.0", "shared": "shared@1.0.0", "cc": "cc@1.0.0",
    "shlex": "shlex@1.0.0", "deep": "deep@1.0.0", "codegen": "codegen@1.0.0", "derive": "derive@1.0.0",
    "spare": "spare@1.0.0",
};
let proc_macro = #{
    "helper@1.0.0": false, "dual@1.0.0": false, "shared@1.0.0": false, "cc@1.0.0": false,
    "shlex@1.0.0": false, "deep@1.0.0": false, "codegen@1.0.0": true, "derive@1.0.0": true,
    "spare@1.0.0": false,
};
let full = #{
    "helper@1.0.0": ["derive@1.0.0", "shared@1.0.0"], "dual@1.0.0": [], "shared@1.0.0": [],
    "cc@1.0.0": ["derive@1.0.0", "shlex@1.0.0", "codegen@1.0.0", "shared@1.0.0"],
    "shlex@1.0.0": ["deep@1.0.0"], "deep@1.0.0": [], "codegen@1.0.0": [], "derive@1.0.0": [],
    "spare@1.0.0": [],
};
let normal_edges = #{
    "helper@1.0.0": ["derive@1.0.0", "shared@1.0.0"], "dual@1.0.0": [], "shared@1.0.0": [],
    "cc@1.0.0": ["derive@1.0.0"], "shlex@1.0.0": [], "deep@1.0.0": [], "codegen@1.0.0": [],
    "derive@1.0.0": [], "spare@1.0.0": [],
};
let build_edges = #{
    "helper@1.0.0": [], "dual@1.0.0": [], "shared@1.0.0": [],
    "cc@1.0.0": ["shlex@1.0.0", "codegen@1.0.0", "shared@1.0.0"], "shlex@1.0.0": ["deep@1.0.0"],
    "deep@1.0.0": [], "codegen@1.0.0": [], "derive@1.0.0": [], "spare@1.0.0": [],
};
let keys = full.keys();
let units = unit_configurations(ctx, by_name, full, normal_edges, build_edges, proc_macro, keys);
if units["helper@1.0.0"] != ["target"] { throw `a normal dependency is a target unit: ${units["helper@1.0.0"]}`; }
if units["dual@1.0.0"] != ["host", "target"] { throw `a crate that is a normal and a build dependency needs both units: ${units["dual@1.0.0"]}`; }
if units["cc@1.0.0"] != ["host"] { throw `a build dependency is host only: ${units["cc@1.0.0"]}`; }
if units["shared@1.0.0"] != ["host", "target"] { throw `a build dependency of a host crate that a target crate also uses needs both units: ${units["shared@1.0.0"]}`; }
if units["shlex@1.0.0"] != ["host"] { throw `a build dependency of a build dependency is host only: ${units["shlex@1.0.0"]}`; }
if units["deep@1.0.0"] != ["host"] { throw `a transitive build dependency is host only: ${units["deep@1.0.0"]}`; }
if units["codegen@1.0.0"] != ["host"] { throw `a build dependency that is a proc macro is host only: ${units["codegen@1.0.0"]}`; }
if units["derive@1.0.0"] != ["host"] { throw `a proc macro is host only: ${units["derive@1.0.0"]}`; }
if units["spare@1.0.0"] != ["target"] { throw `a lock root stays a target unit: ${units["spare@1.0.0"]}`; }
let linked = #{
    "dual@1.0.0": #{ host: "dual-host.rlib", target: "dual-target.rlib" },
    "cc@1.0.0": #{ host: "cc-host.rlib" },
    "derive@1.0.0": #{ host: "derive-host.so" },
};
if configuration_artifact(linked["dual@1.0.0"], proc_macro, "dual@1.0.0", "host") != "dual-host.rlib" { throw "wrong host artifact"; }
if configuration_artifact(linked["derive@1.0.0"], proc_macro, "derive@1.0.0", "target") != "derive-host.so" { throw "a target unit links the host proc macro"; }
let rejected = false;
try { configuration_artifact(linked["cc@1.0.0"], proc_macro, "cc@1.0.0", "target"); } catch { rejected = true; }
if !rejected { throw "a host-only dependency must not link into a target unit"; }
let gated = #{ "target": #{ "cfg(windows)": #{ dependencies: #{ "windows-sys": "1" } } } };
if !manifest_declares_dependencies(gated) { throw "a manifest whose only dependencies are target gated still declares them"; }
if manifest_declares_dependencies(#{ "dependencies": #{ shlex: "1" } }) != true { throw "a manifest with dependencies declares them"; }
if manifest_declares_dependencies(#{}) { throw "a manifest without dependencies does not declare them"; }
let renamed = #{ manifest: #{ lib: #{ name: "webpki" } }, src: #{ name: "rustls-webpki", version: "0.103.15" } };
if unit_crate_name(renamed) != "webpki" { throw `a renamed lib must name its artifact after the crate rustc sees: ${unit_crate_name(renamed)}`; }
if unit_crate_name(#{ manifest: #{}, src: #{ name: "rustls-webpki", version: "0.103.15" } }) != "rustls_webpki" { throw "a package without a lib name uses its own"; }
"#;
	engine.eval::<()>(&format!("{cell}\n{checks}")).unwrap();
}

#[test]
fn feature_propagation_from_root() {
	let cell = crate::std_cells::cell_script("rust").expect("rust cell is embedded");
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	let ctx = Map::new();
	let mut scope = rhai::Scope::new();
	scope.push("ctx", ctx);
	let tail = r#"
let order = ["clap@4.6.7", "clap_builder@4.6.7"];
let names = #{ "clap@4.6.7": "clap", "clap_builder@4.6.7": "clap_builder" };
let tables = #{
  "clap@4.6.7": #{ "default": ["std"], "std": ["clap_builder/std"] },
  "clap_builder@4.6.7": #{ "std": [] },
};
let defaults = #{ "clap@4.6.7": ["std"], "clap_builder@4.6.7": [] };
let edges = #{
  "clap@4.6.7": [#{ dep: "clap_builder@4.6.7", spec: #{ "default-features": false }, dep_defaults: ["std"], prefixes: ["clap_builder/"] }],
  "clap_builder@4.6.7": [],
};
let requests = #{ "clap": #{ defaults: true, features: [] } };
let enabled = feature_closure(order, requests, names, edges, defaults, tables);
enabled["clap_builder@4.6.7"].contains("std")
"#;
	let program = format!("{cell}\n{tail}");
	let propagated = engine.eval_with_scope::<bool>(&mut scope, &program).unwrap();
	assert!(propagated, "clap_builder should inherit std via clap");
}

#[test]
fn workspace_root_requests_reads_members() {
	let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.unwrap()
		.parent()
		.unwrap()
		.to_path_buf();
	let cell = crate::std_cells::cell_script("rust").expect("rust cell is embedded");
	let mut engine = rhai::Engine::new();
	engine.set_max_expr_depths(128, 128);
	register_memo(&mut engine);
	let glob_root = repo.clone();
	engine.register_fn(
		"glob",
		move |_ctx: &mut Map, pattern: &str| -> Result<rhai::Array, Box<EvalAltResult>> {
			let hits =
				crate::glob::expand_glob(&glob_root, pattern).map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })?;
			Ok(hits
				.into_iter()
				.map(|p| Dynamic::from(p.to_string_lossy().into_owned()))
				.collect())
		},
	);
	let read_root = repo.clone();
	engine.register_fn(
		"read_file",
		move |_ctx: &mut Map, path: &str| -> Result<String, Box<EvalAltResult>> {
			std::fs::read_to_string(read_root.join(path)).map_err(|e| -> Box<EvalAltResult> { e.to_string().into() })
		},
	);
	engine.register_fn("toml_decode", |text: &str| -> Result<Map, Box<EvalAltResult>> {
		crate::rhai_rt::toml_decode(text).map_err(Into::into)
	});
	let mut scope = rhai::Scope::new();
	let mut ctx = Map::new();
	ctx.insert(
		"cell_config".into(),
		Dynamic::from(rhai::Map::from_iter([("mode".into(), Dynamic::from("cargo".to_string()))])),
	);
	scope.push("ctx", ctx);
	let tail = r#"
let requests = workspace_root_requests(ctx);
let has_clap = requests.contains("clap");
let defaults = if has_clap { requests["clap"]["defaults"] } else { false };
let has_derive = if has_clap { requests["clap"]["features"].contains("derive") } else { false };
#{ clap: has_clap, defaults: defaults, derive: has_derive }
"#;
	let program = format!("{cell}\n{tail}");
	let result: Map = engine.eval_with_scope(&mut scope, &program).unwrap();
	eprintln!("requests: {result:?}");
	assert_eq!(result.get("clap").and_then(|v| v.as_bool().ok()), Some(true));
	assert_eq!(result.get("derive").and_then(|v| v.as_bool().ok()), Some(true));
}
