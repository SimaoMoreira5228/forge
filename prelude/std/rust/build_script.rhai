fn emit_build_script(ctx, rustc, manifest, enabled, root, name, version, edition, build_deps, link_deps, package_files, extra_inputs) {
    let build_name = if manifest["package"] == () || manifest["package"]["build"] == () { "build.rs" } else { manifest["package"]["build"] };
    if build_name == false { return ""; }
    if build_name == true { build_name = "build.rs"; }
    let build_files = ctx.glob(if root == "" { build_name } else { `${root}/${build_name}` });
    if build_files.len() == 0 { return ""; }
    let pkg_name = crate_name(#{ name: name });
    let build_output = build_output_dir(ctx, name, version);
    let build_exe = `${build_output}/script`;
    let build_args = ["--crate-type", "bin", "--crate-name", `${pkg_name}_build`, "--edition", edition, "-C", `metadata=${pkg_name}_build-${version}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", `dependency=${rust_host_deps_dir(ctx)}`] + feature_cfg_args(manifest, enabled) + rust_crate_profile_args(rust_host_profile(ctx), false);
    let build_inputs = package_files + [build_files[0]] + extra_inputs;
    let build_execution_deps = [];
    let environment_files = #{ };
    let environment_inputs = [];
    for input in extra_inputs {
        if input.starts_with("forge-out/") { build_execution_deps.push(input); }
    }
    for dep in build_deps {
        build_args.push("--extern");
        build_args.push(`${dep["name"]}=${dep["path"]}`);
        build_execution_deps.push(dep["path"]);
    }
    for dep in link_deps {
        let link = dep["links"];
        link.replace("-", "_");
        link = link.to_upper();
        let metadata_file = #{
            path: `${dep["dir"]}/.forge-build-output`,
            line_prefix: "cargo:",
            ignored_keys: ["rerun-if-changed", "rerun-if-env-changed", "warning", "rustc-link-lib", "rustc-link-search", "rustc-flags", "rustc-cfg", "rustc-env"],
        };
        environment_files[`DEP_${link}_`] = metadata_file;
        environment_files[`CARGO_DEP_${link}_`] = metadata_file;
        environment_inputs.push(dep["dir"]);
    }
    let compile_env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, name, version));
    let manifest_path = if manifest["forge-manifest-path"] == () { if root == "" { "Cargo.toml" } else { `${root}/Cargo.toml` } } else { manifest["forge-manifest-path"] };
    compile_env["CARGO_MANIFEST_DIR"] = if root == "" { "FORGE_EXEC_ROOT" } else { `FORGE_EXEC_ROOT/${root}` };
    compile_env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${manifest_path}`;
    compile_env["HOST"] = target_for(ctx);
    compile_env["TARGET"] = target_for(ctx);
    compile_env["RUSTC"] = rustc;
    ctx.action(#{
        name: "rustc build script " + name + "@" + version,
        configuration: "host",
        command: rustc,
        args: build_args + ["-o", build_exe, build_files[0]],
        inputs: build_inputs,
        execution_deps: build_execution_deps,
        outputs: [build_exe],
        env: compile_env,
        toolchain_id: ctx.tool_id(),
    });
    let run_env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, name, version));
    run_env["OUT_DIR"] = `FORGE_EXEC_ROOT/${build_output}`;
    run_env["CARGO_MANIFEST_DIR"] = compile_env["CARGO_MANIFEST_DIR"];
    run_env["CARGO_MANIFEST_PATH"] = compile_env["CARGO_MANIFEST_PATH"];
    for feature_name in feature_env(enabled).keys() { run_env[feature_name] = "1"; }
    run_env["NUM_JOBS"] = "1";
    run_env["OPT_LEVEL"] = `${rust_host_profile(ctx).opt_level}`;
    run_env["DEBUG"] = if rust_host_profile(ctx).is_debug { "true" } else { "false" };
    run_env["PROFILE"] = ctx.profile.name;
    run_env["RUSTDOC"] = "rustdoc";
    run_env["HOST"] = target_for(ctx);
    run_env["TARGET"] = target_for(ctx);
    run_env["RUSTC"] = rustc;
    run_env["CARGO_CFG_TARGET_ARCH"] = ctx.platform_arch;
    run_env["CARGO_CFG_TARGET_OS"] = ctx.platform_os;
    run_env["CARGO_CFG_TARGET_ENV"] = ctx.platform_abi;
    run_env["CARGO_CFG_TARGET_VENDOR"] = "unknown";
    run_env["CARGO_CFG_TARGET_POINTER_WIDTH"] = if ctx.platform_arch == "x86_64" || ctx.platform_arch == "aarch64" { "64" } else { "32" };
    run_env["CARGO_CFG_TARGET_ENDIAN"] = "little";
    run_env["CARGO_CFG_TARGET_FEATURE"] = if ctx.platform_arch == "x86_64" { "fxsr,sse,sse2" } else if ctx.platform_arch == "aarch64" { "neon" } else { "" };
    if ctx.platform_os == "linux" || ctx.platform_os == "darwin" { run_env["CARGO_CFG_UNIX"] = "1"; }
    run_env["CRUNCHY_LIB_SUFFIX"] = "/lib.rs";
    ctx.action(#{
        name: "run build script " + name + "@" + version,
        configuration: "host",
        command: `FORGE_EXEC_ROOT/${build_exe}`,
        args: [],
        inputs: package_files + extra_inputs,
        execution_deps: [build_exe] + environment_inputs,
        outputs: [build_output + "/"],
        workdir: root,
        stdout: `${build_output}/.forge-build-output`,
        environment_files: environment_files,
        env: run_env,
        toolchain_id: ctx.tool_id(),
    });
    build_output
}

fn build_output_argument_files(build_output) {
    let output_file = `${build_output}/.forge-build-output`;
    let files = [];
    for directive in ["cfg", "link-lib", "link-search"] {
        let flag = if directive == "cfg" { "--cfg" } else if directive == "link-lib" { "-l" } else { "-L" };
        files.push(#{ path: output_file, prefix: `cargo:rustc-${directive}=`, flag: flag, root_marker: "forge-out/" });
        files.push(#{ path: output_file, prefix: `cargo::rustc-${directive}=`, flag: flag, root_marker: "forge-out/" });
    }
    files
}

fn authors_env(authors) {
    let result = "";
    for author in authors {
        if result != "" { result += ":"; }
        result += author;
    }
    result
}

fn manifest_string(value) {
    if value == () { "" } else if value.type_of() == "string" { value } else { value.to_string() }
}

fn cargo_pkg_env(manifest, name, version) {
    let version = if version.type_of() == "string" { version } else { "0.0.0" };
    let pkg = manifest["package"];
    let env = #{};
    env["CARGO_PKG_NAME"] = name;
    env["CARGO_PKG_VERSION"] = version;
    let release = version.split("+")[0].split("-");
    let parts = release[0].split(".");
    if parts.len() != 3 { throw `Rust package version must be major.minor.patch: ${version}`; }
    env["CARGO_PKG_VERSION_MAJOR"] = parts[0];
    env["CARGO_PKG_VERSION_MINOR"] = parts[1];
    env["CARGO_PKG_VERSION_PATCH"] = parts[2];
    let prerelease = version.split("+")[0];
    env["CARGO_PKG_VERSION_PRE"] = if prerelease.contains("-") { prerelease.sub_string(prerelease.index_of("-") + 1) } else { "" };
    env["CARGO_PKG_AUTHORS"] = if pkg == () || pkg["authors"] == () { "" } else { authors_env(pkg["authors"]) };
    env["CARGO_PKG_DESCRIPTION"] = if pkg == () { "" } else { manifest_string(pkg["description"]) };
    env["CARGO_PKG_HOMEPAGE"] = if pkg == () { "" } else { manifest_string(pkg["homepage"]) };
    env["CARGO_PKG_REPOSITORY"] = if pkg == () { "" } else { manifest_string(pkg["repository"]) };
    env["CARGO_PKG_LICENSE"] = if pkg == () { "" } else { manifest_string(pkg["license"]) };
    env["CARGO_PKG_LICENSE_FILE"] = if pkg == () { "" } else { manifest_string(pkg["license-file"]) };
    env["CARGO_PKG_RUST_VERSION"] = if pkg == () { "" } else { manifest_string(pkg["rust-version"]) };
    env["CARGO_PKG_README"] = if pkg == () { "" } else { manifest_string(pkg["readme"]) };
    env["CARGO_MANIFEST_LINKS"] = if pkg == () { "" } else { manifest_string(pkg["links"]) };
    env
}

fn merge_env(base, extra) {
    for key in extra.keys() {
        base[key] = extra[key];
    }
    base
}

fn rust_metadata(ctx) {
    let metadata = if ctx["metadata"] == () { #{} } else { ctx["metadata"] };
    let rust = if metadata["rust"] == () { #{} } else { metadata["rust"] };
    if rust.type_of() != "map" { throw "metadata.rust must be a table"; }
    for key in rust.keys() {
        if !["root", "crate_root", "name", "edition", "package", "build", "manifest", "manifest_path", "externs", "dependencies", "build-dependencies", "features", "enabled_features"].contains(key) {
            throw `unknown metadata.rust key: ${key}`;
        }
    }
    rust
}

fn workspace_crate_root(ctx) {
    let metadata = rust_metadata(ctx);
    if metadata["root"] != () { return rust_path("", metadata["root"]); }
    if ctx.srcs.len() == 0 { return ""; }
    let first = ctx.srcs[0];
    let index = first.index_of("/src/");
    if index < 0 { return ""; }
    rust_path("", first.sub_string(0, index))
}

fn workspace_crate_manifest(ctx) {
    let metadata = rust_metadata(ctx);
    let mode = rust_mode(ctx["cell_config"]);
    let manifest = #{};
    if mode == "cargo" {
        if metadata["manifest"] != () {
            manifest = metadata["manifest"];
        } else {
            let path = rust_path(workspace_crate_root(ctx), "Cargo.toml");
            if ctx.glob(path).len() == 0 { throw `cargo mode requires package manifest: ${path}`; }
            manifest = toml_decode(ctx.read_file(path));
            let root = toml_decode(ctx.read_file("Cargo.toml"));
            manifest = cargo_inherit(manifest, if root["workspace"] == () { #{} } else { root["workspace"] });
        }
    } else if metadata["manifest"] != () {
        throw "native Rust metadata uses package/dependencies, not a Cargo manifest";
    }
    let pkg = if manifest["package"] == () { #{} } else { manifest["package"] };
    if metadata["package"] != () { pkg = merge_env(pkg, metadata["package"]); }
    if metadata["edition"] != () { pkg["edition"] = metadata["edition"]; }
    if metadata["build"] != () { pkg["build"] = metadata["build"]; }
    if pkg["name"] == () { pkg["name"] = ctx.name; }
    if pkg["version"] == () { pkg["version"] = "0.0.0"; }
    if pkg["edition"] == () { pkg["edition"] = if mode == "native" { "2024" } else { "2015" }; }
    if mode == "native" && pkg["build"] == () { pkg["build"] = false; }
    manifest["package"] = pkg;
    for key in ["dependencies", "build-dependencies", "features"] {
        if metadata[key] != () { manifest[key] = metadata[key]; }
    }
    manifest
}

fn workspace_package_version(ctx) {
    workspace_crate_manifest(ctx)["package"]["version"]
}

fn workspace_manifest_path(ctx) {
    let metadata = rust_metadata(ctx);
    if metadata["manifest_path"] != () { return rust_path("", metadata["manifest_path"]); }
    if rust_mode(ctx["cell_config"]) == "cargo" { rust_path(workspace_crate_root(ctx), "Cargo.toml") }
    else { "FORGE.toml" }
}

fn workspace_crate_env(ctx) {
    let manifest = workspace_crate_manifest(ctx);
    let pkg = manifest["package"];
    let root = workspace_crate_root(ctx);
    let env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, pkg["name"], pkg["version"]));
    env["CARGO_CRATE_NAME"] = crate_name(ctx);
    env["CARGO_MANIFEST_DIR"] = if root == "" { "FORGE_EXEC_ROOT" } else { `FORGE_EXEC_ROOT/${root}` };
    env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${workspace_manifest_path(ctx)}`;
    env
}

fn string_env(input) {
    let output = #{ };
    for key in input.keys() {
        output[key] = if input[key] == () { "" } else { input[key].to_string() };
    }
    output
}
