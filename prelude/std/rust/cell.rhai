fn rustc_for(ctx) {
    let path = ctx.bin("rustc");
    if path == "" {
        throw `toolchain provides no rustc`;
    }
    path;
}

fn edition_for(ctx) {
    let std_val = ctx.standard;
    if std_val == "" {
        "2024"
    } else {
        std_val
    }
}

fn crate_name(ctx) {
    let name = ctx.name;
    name.replace("-", "_");
    name
}

fn rlib_path(ctx, component_name) {
    let path = ctx.lib_path(component_name);
    path.replace(".a", ".rlib");
    path
}

fn dependency_key(src) {
    `${src.name}@${src.version}`
}

fn proc_macro_dylib_suffix(ctx) {
    if ctx.platform_os == "darwin" { "dylib" }
    else if ctx.platform_os == "windows" { "dll" }
    else { "so" }
}

fn target_for(ctx) {
    if ctx.platform_os == "linux" { `${ctx.platform_arch}-unknown-linux-${ctx.platform_abi}` }
    else if ctx.platform_os == "darwin" { `${ctx.platform_arch}-apple-darwin` }
    else if ctx.platform_os == "windows" { `${ctx.platform_arch}-pc-windows-msvc` }
    else { `${ctx.platform_arch}-unknown-${ctx.platform_os}` }
}

fn section_dep_names(section) {
    let names = [];
    for name in section.keys() {
        let spec = section[name];
        let package_name = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { name };
        names.push(package_name);
    }
    names
}

fn active_dep_sections(ctx, manifest) {
    let sections = [];
    let deps = manifest["dependencies"];
    if deps != () { sections.push(deps); }
    let build = manifest["build-dependencies"];
    if build != () { sections.push(build); }
    let targets = manifest["target"];
    if targets != () {
        for key in targets.keys() {
            if !ctx.platform_matches(key) { continue; }
            let section = targets[key];
            let targeted = section["dependencies"];
            if targeted != () { sections.push(targeted); }
            let targeted_build = section["build-dependencies"];
            if targeted_build != () { sections.push(targeted_build); }
        }
    }
    sections
}

fn active_dep_names(ctx, manifest) {
    let names = [];
    for section in active_dep_sections(ctx, manifest) {
        names += section_dep_names(section);
    }
    names
}

fn dependency_entry(ctx, manifest, package_name) {
    for section in active_dep_sections(ctx, manifest) {
        for dependency_name in section.keys() {
            let spec = section[dependency_name];
            let resolved = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { dependency_name };
            if resolved == package_name {
                return #{ name: dependency_name, spec: spec };
            }
        }
    }
    ()
}

fn dependency_crate_name(ctx, manifest, package_name, fallback) {
    let entry = dependency_entry(ctx, manifest, package_name);
    if entry == () { return fallback; }
    let name = entry["name"];
    name.replace("-", "_");
    name
}

fn edition_of(manifest) {
    let pkg = manifest["package"];
    if pkg == () { return "2015"; }
    let edition = pkg["edition"];
    if edition == () || edition == "" { "2015" } else { edition }
}

fn source_meta(ctx) {
    let meta = #{};
    let adjacency = #{};
    let full_adjacency = #{};
    let keys = [];
    for src in ctx.fetched_sources {
        let key = dependency_key(src);
        let manifest = if ctx.glob(`${src.root}/Cargo.toml`).len() == 0 {
            #{ "package": #{}, "lib": #{}, "features": #{} }
        } else {
            toml_decode(ctx.read_file(`${src.root}/Cargo.toml`))
        };
        let lib = manifest["lib"];
        let proc_macro = lib != () && lib["proc-macro"] == true;
        let suffix = if proc_macro { proc_macro_dylib_suffix(ctx) } else { "rlib" };
        let crate = src.name;
        crate.replace("-", "_");
        meta[key] = #{
            src: src,
            manifest: manifest,
            edition: edition_of(manifest),
            artifact: `forge-out/lib/deps/lib${crate}-${src.version}.${suffix}`,
            active: true,
        };
        keys.push(key);
    }
    for key in keys {
        let src = meta[key]["src"];
        let names = active_dep_names(ctx, meta[key]["manifest"]);
        let deps = [];
        let full = [];
        for dep_str in src.dependencies {
            let dep_name = dep_str.split(" ")[0];
            let dep_version = if dep_str.contains(" ") { dep_str.split(" ")[1] } else { "" };
            let package_name = if dep_str.contains(" package ") {
                dep_str.split(" package ")[1].split(" ")[0]
            } else {
                dep_name
            };
            for candidate in ctx.fetched_sources {
                if candidate.name == package_name && (dep_version == "" || candidate.version == dep_version) {
                    let dep_key = dependency_key(candidate);
                    full.push(dep_key);
                    if names.len() == 0 || names.contains(package_name) {
                        deps.push(dep_key);
                    }
                    break;
                }
            }
        }
        adjacency[key] = deps;
        full_adjacency[key] = full;
    }
    #{ meta: meta, adjacency: adjacency, full: full_adjacency, keys: keys }
}

fn append_features(enabled, features) {
    if features == () { return enabled; }
    for feature in features {
        if !enabled.contains(feature) { enabled.push(feature); }
    }
    enabled
}

fn expand_features(feature_table, features) {
    let enabled = features;
    if feature_table == () { return enabled; }
    let index = 0;
    while index < enabled.len() {
        let feature = enabled[index];
        index += 1;
        let members = feature_table[feature];
        if members == () { continue; }
        for member in members {
            if member.starts_with("dep:") || member.contains("/") { continue; }
            if !enabled.contains(member) { enabled.push(member); }
        }
    }
    enabled
}

fn collect_requests(section, requests) {
    for name in section.keys() {
        let spec = section[name];
        let package_name = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { name };
        let entry = if requests.contains(package_name) { requests[package_name] } else { #{ defaults: false, features: [] } };
        if spec.type_of() != "map" || spec["default-features"] != false { entry["defaults"] = true; }
        if spec.type_of() == "map" && spec["features"] != () {
            entry["features"] = append_features(entry["features"], spec["features"]);
        }
        requests[package_name] = entry;
    }
    requests
}

fn workspace_root_requests(ctx) {
    let requests = #{};
    let root = if ctx.glob("Cargo.toml").len() == 0 { #{} } else { toml_decode(ctx.read_file("Cargo.toml")) };
    let workspace = root["workspace"];
    if workspace != () && workspace["dependencies"] != () {
        requests = collect_requests(workspace["dependencies"], requests);
    }
    if workspace != () && workspace["members"] != () {
        for member in workspace["members"] {
            for manifest_path in ctx.glob(`${member}/Cargo.toml`) {
                let manifest = toml_decode(ctx.read_file(manifest_path));
                if manifest["dependencies"] != () {
                    requests = collect_requests(manifest["dependencies"], requests);
                }
                if manifest["build-dependencies"] != () {
                    requests = collect_requests(manifest["build-dependencies"], requests);
                }
            }
        }
    }
    if root["dependencies"] != () { requests = collect_requests(root["dependencies"], requests); }
    if root["build-dependencies"] != () { requests = collect_requests(root["build-dependencies"], requests); }
    requests
}

fn feature_sets(ctx, meta, adjacency, order, requests) {
    let enabled = #{};
    for key in order {
        enabled[key] = [];
    }
    for key in order {
        let name = meta[key]["src"].name;
        if requests.contains(name) {
            let request = requests[name];
            let dep_features = meta[key]["manifest"]["features"];
            if request["defaults"] && dep_features != () {
                enabled[key] = append_features(enabled[key], dep_features["default"]);
            }
            enabled[key] = append_features(enabled[key], request["features"]);
        }
    }
    for key in order {
        let manifest = meta[key]["manifest"];
        let feature_table = manifest["features"];
        enabled[key] = expand_features(feature_table, enabled[key]);
        for dep_key in adjacency[key] {
            let dep_name = meta[dep_key]["src"].name;
            let entry = dependency_entry(ctx, manifest, dep_name);
            if entry == () { continue; }
            let spec = entry["spec"];
            if spec.type_of() != "map" || spec["default-features"] != false {
                let dep_features = meta[dep_key]["manifest"]["features"];
                if dep_features != () {
                    enabled[dep_key] = append_features(enabled[dep_key], dep_features["default"]);
                }
            }
            if spec.type_of() == "map" && spec["features"] != () {
                enabled[dep_key] = append_features(enabled[dep_key], spec["features"]);
            }
            let prefixes = [`${entry["name"]}/`, `${dep_name}/`, `${entry["name"]}?/`, `${dep_name}?/`];
            let parent_features = manifest["features"];
            if parent_features != () {
                for parent_feature in parent_features.keys() {
                    if !enabled[key].contains(parent_feature) { continue; }
                    for member in parent_features[parent_feature] {
                        for prefix in prefixes {
                            if member.starts_with(prefix) {
                                enabled[dep_key] = append_features(enabled[dep_key], [member.sub_string(prefix.len(), member.len() - prefix.len())]);
                            }
                        }
                    }
                }
            }
        }
    }
    enabled
}

fn manifest_has_dependency(manifest, name) {
    for section_name in ["dependencies", "build-dependencies"] {
        let section = manifest[section_name];
        if section == () { continue; }
        for dependency_name in section.keys() {
            let spec = section[dependency_name];
            let resolved = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { dependency_name };
            if resolved == name { return true; }
        }
    }
    false
}

fn feature_cfg_args(manifest, enabled) {
    let feature_table = manifest["features"];
    let deps = manifest["dependencies"];
    let args = [];
    for feature in enabled {
        if feature.starts_with("dep:") || feature.contains("/") { continue; }
        let members = if feature_table == () { () } else { feature_table[feature] };
        if members != () {
            let valid = true;
            for member in members {
                let dep_spec = if deps == () { () } else { deps[member] };
                if dep_spec != () && dep_spec["optional"] == true && !manifest_has_dependency(manifest, member) {
                    valid = false;
                }
            }
            if !valid { continue; }
        }
        let dep_spec = if deps == () { () } else { deps[feature] };
        if dep_spec != () && dep_spec["optional"] == true && !manifest_has_dependency(manifest, feature) { continue; }
        args.push("--cfg");
        args.push(`feature="${feature}"`);
    }
    args
}

fn feature_env(enabled) {
    let env = #{ };
    for feature in enabled {
        if feature.starts_with("dep:") || feature.contains("/") { continue; }
        let key = feature;
        key.replace("-", "_");
        env["CARGO_FEATURE_" + key.to_upper()] = "1";
    }
    env
}

fn dependency_artifacts(meta, dep_keys) {
    let artifacts = [];
    for dep_key in dep_keys {
        artifacts.push(meta[dep_key]["artifact"]);
    }
    artifacts
}

fn build_dependency_externs(meta, manifest) {
    let externs = [];
    let section = manifest["build-dependencies"];
    if section == () { return externs; }
    for name in section.keys() {
        let spec = section[name];
        let package_name = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { name };
        for key in meta.keys() {
            let src = meta[key]["src"];
            if meta[key]["active"] == false || src.name != package_name { continue; }
            let extern_name = name;
            extern_name.replace("-", "_");
            let dep_manifest = meta[key]["manifest"];
            let links = if dep_manifest["package"] == () || dep_manifest["package"]["links"] == () { "" } else { dep_manifest["package"]["links"] };
            externs.push(#{
                name: extern_name,
                path: meta[key]["artifact"],
                links: links,
                dir: `forge-out/build/${src.name}-${src.version}`,
            });
            break;
        }
    }
    externs
}

fn emit_build_script(ctx, rustc, manifest, enabled, root, name, version, edition, build_deps, package_files, extra_inputs) {
    let build_name = if manifest["package"] == () || manifest["package"]["build"] == () { "build.rs" } else { manifest["package"]["build"] };
    let build_files = ctx.glob(`${root}/${build_name}`);
    if build_files.len() == 0 { return ""; }
    let pkg_name = crate_name(#{ name: name });
    let build_output = `forge-out/build/${name}-${version}`;
    let build_exe = `${build_output}/script`;
    let build_args = ["--crate-type", "bin", "--crate-name", `${pkg_name}_build`, "--edition", edition, "-C", `metadata=${pkg_name}_build-${version}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", "dependency=forge-out/lib/deps"] + feature_cfg_args(manifest, enabled);
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
        if dep["links"] != "" {
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
    }
    let compile_env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, name, version));
    compile_env["CARGO_MANIFEST_DIR"] = `FORGE_EXEC_ROOT/${root}`;
    compile_env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${root}/Cargo.toml`;
    compile_env["HOST"] = target_for(ctx);
    compile_env["TARGET"] = target_for(ctx);
    compile_env["RUSTC"] = rustc;
    ctx.action(#{
        name: "rustc build script " + name,
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
    run_env["CARGO_MANIFEST_DIR"] = `FORGE_EXEC_ROOT/${root}`;
    run_env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${root}/Cargo.toml`;
    for feature_name in feature_env(enabled).keys() { run_env[feature_name] = "1"; }
    run_env["NUM_JOBS"] = "1";
    run_env["OPT_LEVEL"] = `${ctx.profile.opt_level}`;
    run_env["DEBUG"] = if ctx.profile.is_debug { "true" } else { "false" };
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
        name: "run build script " + name,
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
    env["CARGO_PKG_VERSION_MAJOR"] = version.split(".")[0];
    env["CARGO_PKG_VERSION_MINOR"] = version.split(".")[1];
    env["CARGO_PKG_VERSION_PATCH"] = version.split(".")[2];
    env["CARGO_PKG_VERSION_PRE"] = "";
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

fn workspace_crate_root(ctx) {
    if ctx.srcs.len() == 0 { return ""; }
    let first = ctx.srcs[0];
    let index = first.index_of("/src/");
    if index < 0 { return ""; }
    first.sub_string(0, index)
}

fn workspace_crate_manifest(ctx) {
    let root = workspace_crate_root(ctx);
    if root == "" || ctx.glob(`${root}/Cargo.toml`).len() == 0 { return #{}; }
    toml_decode(ctx.read_file(`${root}/Cargo.toml`))
}

fn workspace_package_version(ctx) {
    let root = if ctx.glob("Cargo.toml").len() == 0 { #{} } else { toml_decode(ctx.read_file("Cargo.toml")) };
    let workspace = root["workspace"];
    if workspace != () && workspace["package"] != () && workspace["package"]["version"] != () {
        workspace["package"]["version"]
    } else {
        "0.0.0"
    }
}

fn workspace_crate_env(ctx) {
    let manifest = workspace_crate_manifest(ctx);
    let name = if manifest["package"] != () { manifest["package"]["name"] } else { ctx.name };
    let declared = if manifest["package"] != () && manifest["package"]["version"] != () { manifest["package"]["version"] } else { () };
    let version = if declared.type_of() == "string" { declared } else { workspace_package_version(ctx) };
    let root = workspace_crate_root(ctx);
    let env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, name, version));
    env["CARGO_CRATE_NAME"] = ctx.name;
    env["CARGO_MANIFEST_DIR"] = `FORGE_EXEC_ROOT/${root}`;
    env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${root}/Cargo.toml`;
    env
}

fn string_env(input) {
    let output = #{ };
    for key in input.keys() {
        output[key] = if input[key] == () { "" } else { input[key].to_string() };
    }
    output
}

fn build(ctx) {
    let rustc = rustc_for(ctx);
    let edition = edition_for(ctx);
    let name = crate_name(ctx);

    let info = source_meta(ctx);
    let meta = info["meta"];
    let adjacency = info["adjacency"];
    let keys = info["keys"];
    let roots = graph_roots(info["full"]);
    if roots.len() == 0 { roots = keys; }
    let reachable = #{};
    for key in graph_reachable(adjacency, roots) {
        reachable[key] = true;
    }
    for key in keys {
        let entry = meta[key];
        entry["active"] = reachable.contains(key);
        meta[key] = entry;
    }
    let features = feature_sets(ctx, meta, adjacency, graph_topo(adjacency), workspace_root_requests(ctx));

    if ctx.fetch_owner {
        for src in ctx.fetched_sources {
            let key = dependency_key(src);
            if meta[key]["active"] == false { continue; }
            let manifest = meta[key]["manifest"];
            let direct = adjacency[key];
            let transitive = graph_transitive(adjacency, key);
            let enabled = features[key];
            let pkg_name = crate_name(#{ name: src.name });
            let source_edition = meta[key]["edition"];
            let crate_type = if manifest["lib"] != () && manifest["lib"]["proc-macro"] == true { "proc-macro" } else { "lib" };
            let crate_root = `${src.root}/src/lib.rs`;
            let out = meta[key]["artifact"];
            let args = ["--crate-type", crate_type, "--crate-name", pkg_name, "--edition", source_edition, "-C", `metadata=${pkg_name}-${src.version}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", "dependency=forge-out/lib/deps"];
            if crate_type == "proc-macro" {
                args.push("--extern");
                args.push("proc_macro");
            }
            args += feature_cfg_args(manifest, enabled);
            let package_files = ctx.glob(`${src.root}/**/*`);
            let inputs = package_files + dependency_artifacts(meta, transitive);
            let execution_deps = dependency_artifacts(meta, direct);
            let build_deps = build_dependency_externs(meta, manifest);
            let build_output = emit_build_script(ctx, rustc, manifest, enabled, src.root, src.name, src.version, source_edition, build_deps, package_files, dependency_artifacts(meta, transitive));
            if build_output != "" {
                execution_deps.push(build_output);
            }
            for dep_key in direct {
                let dep_src = meta[dep_key]["src"];
                let dep_out = meta[dep_key]["artifact"];
                let dep_crate = dependency_crate_name(ctx, manifest, dep_src.name, crate_name(#{ name: dep_src.name }));
                args.push("--extern");
                args.push(`${dep_crate}=${dep_out}`);
                execution_deps.push(dep_out);
            }
            args += ["-o", out, crate_root];
            let dep_env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, src.name, src.version));
            dep_env["CARGO_CRATE_NAME"] = pkg_name;
            dep_env["CARGO_MANIFEST_DIR"] = `FORGE_EXEC_ROOT/${src.root}`;
            dep_env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${src.root}/Cargo.toml`;
            if build_output != "" {
                dep_env["OUT_DIR"] = `FORGE_EXEC_ROOT/forge-out/build/${src.name}-${src.version}`;
                dep_env["CRUNCHY_LIB_SUFFIX"] = "/lib.rs";
            }
            let argument_files = if build_output != "" { build_output_argument_files(build_output) } else { [] };
            ctx.action(#{
                name: "rustc dependency " + src.name,
                command: rustc,
                args: args,
                inputs: inputs,
                execution_deps: execution_deps,
                outputs: [out],
                argument_files: argument_files,
                env: dep_env,
                toolchain_id: ctx.tool_id(),
            });
        }
    }

    let external_inputs = [];
    let external_externs = [];
    for src in ctx.fetched_sources {
        let key = dependency_key(src);
        if meta[key]["active"] == false { continue; }
        let out = meta[key]["artifact"];
        let crate_n = crate_name(#{ name: src.name });
        external_inputs.push(out);
        external_externs.push(`${crate_n}=${out}`);
    }

    let crate_env = workspace_crate_env(ctx);
    let crate_root_dir = workspace_crate_root(ctx);
    let crate_manifest = workspace_crate_manifest(ctx);
    let crate_version = if crate_manifest["package"] != () && crate_manifest["package"]["version"].type_of() == "string" { crate_manifest["package"]["version"] } else { workspace_package_version(ctx) };
    let crate_entry = if ctx.kind == "library" { "src/lib.rs" } else { "src/main.rs" };
    let crate_root_file = if crate_root_dir != "" && ctx.glob(`${crate_root_dir}/${crate_entry}`).len() > 0 {
        `${crate_root_dir}/${crate_entry}`
    } else if ctx.srcs.len() > 0 {
        ctx.srcs[0]
    } else {
        throw `component ${ctx.label} declares no crate root`;
    };
    let build_output = emit_build_script(ctx, rustc, crate_manifest, [], crate_root_dir, ctx.name, crate_version, edition, build_dependency_externs(meta, crate_manifest), ctx.glob(`${crate_root_dir}/**/*`), ctx.hdrs + external_inputs);
    if build_output != "" {
        crate_env = merge_env(crate_env, #{ "OUT_DIR": `FORGE_EXEC_ROOT/${build_output}` });
    }
    let dep_inputs = ctx.srcs + ctx.hdrs;
    if build_output != "" { dep_inputs.push(build_output); }
    let execution_deps = ctx.dep_archives + external_inputs;
    for dep in ctx.dep_artifacts {
        execution_deps.push(dep[1]);
    }
    if build_output != "" { execution_deps.push(build_output); }
    let link_args = [];
    for src in ctx.fetched_sources {
        let key = dependency_key(src);
        if meta[key]["active"] == false { continue; }
        let pkg = meta[key]["manifest"]["package"];
        if pkg == () || pkg["links"] == () { continue; }
        let dep_build_output = `forge-out/build/${src.name}-${src.version}`;
        let output_file = `${dep_build_output}/.forge-build-output`;
        for directive in ["link-lib", "link-search"] {
            let flag = if directive == "link-lib" { "-l" } else { "-L" };
            link_args.push(#{ path: output_file, prefix: `cargo:rustc-${directive}=`, flag: flag, root_marker: "forge-out/" });
            link_args.push(#{ path: output_file, prefix: `cargo::rustc-${directive}=`, flag: flag, root_marker: "forge-out/" });
        }
        execution_deps.push(dep_build_output);
    }
    let crate_argument_files = if build_output != "" { link_args + build_output_argument_files(build_output) } else { link_args };

    if ctx.kind == "library" {
        if ctx.srcs.len() == 0 {
            throw `library ${ctx.label} declares no sources`;
        }
        let out = rlib_path(ctx, ctx.name);
        let args = ["--crate-type", "lib", "--crate-name", name, "--edition", edition, "-C", `metadata=${name}-${ctx.profile.name}`, "-L", "dependency=forge-out/lib", "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", "dependency=forge-out/lib/deps"];
        for dep in ctx.dep_artifacts {
            args.push("--extern");
            args.push(`${dep[0]}=${dep[1]}`);
        }
        for ext in external_externs {
            args.push("--extern");
            args.push(ext);
        }
        args += ["-o", out, crate_root_file];
        ctx.action(#{
            name: "rustc " + ctx.label + " -> " + out,
            command: rustc,
            args: args,
            inputs: dep_inputs,
            execution_deps: execution_deps,
            outputs: [out],
            argument_files: crate_argument_files,
            env: crate_env,
            toolchain_id: ctx.tool_id(),
        });
        return;
    }

    let out_dir = if ctx.kind == "test" { "forge-out/test" } else { "forge-out/bin" };
    let exe = `${out_dir}/${ctx.profile.name}/${ctx.name}`;
    let args = ["--crate-type", "bin", "--crate-name", name, "--edition", edition, "-C", `metadata=${name}-${ctx.profile.name}`, "-L", "dependency=forge-out/lib", "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", "dependency=forge-out/lib/deps"];
    for dep in ctx.dep_artifacts {
        args.push("--extern");
        args.push(`${dep[0]}=${dep[1]}`);
    }
    for ext in external_externs {
        args.push("--extern");
        args.push(ext);
    }
    args += ["-o", exe, crate_root_file];
    let outputs = [exe];
    ctx.action(#{
        name: "rustc " + ctx.label + " -> " + outputs[0],
        command: rustc,
        args: args,
        inputs: dep_inputs,
        execution_deps: execution_deps,
        outputs: outputs,
        argument_files: crate_argument_files,
        env: crate_env,
        toolchain_id: ctx.tool_id(),
    });

    if ctx.kind == "test" {
        ctx.action(#{
            name: "run " + ctx.label,
            command: outputs[0],
            args: ctx.run_args,
            inputs: outputs + ctx.data,
            outputs: [],
            env: crate_env,
            toolchain_id: "",
        });
    }
}
