fn rustc_for(ctx) {
    let path = ctx.bin("rustc");
    if path == "" {
        throw `toolchain provides no rustc`;
    }
    path;
}

fn edition_for(ctx) {
    let metadata = rust_metadata(ctx);
    let manifest = workspace_crate_manifest(ctx);
    let edition = if metadata["edition"] != () { metadata["edition"] }
        else if rust_mode(ctx["cell_config"]) == "cargo" || (metadata["package"] != () && metadata["package"]["edition"] != ()) { manifest["package"]["edition"] }
        else if ctx.standard != "" { ctx.standard }
        else { "2024" };
    if !["2015", "2018", "2021", "2024"].contains(edition) { throw `unsupported Rust edition: ${edition}`; }
    edition
}

fn crate_name(ctx) {
    let metadata = rust_metadata(ctx);
    rust_name(if metadata["name"] == () { ctx.name } else { metadata["name"] })
}

fn workspace_extern_name(ctx, name) {
    let aliases = rust_metadata(ctx)["externs"];
    rust_name(if aliases != () && aliases[name] != () { aliases[name] } else { name })
}

fn rust_debuginfo(info) {
    if info == "limited" { "debuginfo=1" }
    else if info == "line-tables-only" { "debuginfo=line-tables-only" }
    else if info == "none" { "debuginfo=0" }
    else { "debuginfo=2" }
}

fn rust_option_args(options) {
    let args = [];
    if options["codegen-units"] != () { args += ["-C", `codegen-units=${options["codegen-units"]}`]; }
    if options["panic"] != () && options["panic"] != "unwind" { args += ["-C", `panic=${options["panic"]}`]; }
    if options["overflow-checks"] != () { args += ["-C", `overflow-checks=${if options["overflow-checks"] { "on" } else { "off" }}`]; }
    if options["split-debuginfo"] != () { args += ["-C", `split-debuginfo=${options["split-debuginfo"]}`]; }
    if options["rustflags"] != () {
        for flag in options["rustflags"] { args.push(flag); }
    }
    args
}

fn rust_crate_profile_args(profile, link) {
    let args = ["-C", `opt-level=${profile.opt_level}`, "-C", rust_debuginfo(profile["debug"])];
    if link && profile.lto == "thin" { args += ["-C", "lto=thin"]; }
    if link && profile.lto == "fat" { args += ["-C", "lto"]; }
    if link && profile.strip != "none" { args += ["-C", `strip=${profile.strip}`]; }
    args += rust_option_args(if profile.options == () { #{} } else { profile.options });
    args
}

fn rust_profile_args(ctx) { rust_crate_profile_args(ctx.profile, true); }

fn rust_host_profile(ctx) {
    if ctx.profile.build == () { ctx.profile } else { ctx.profile.build }
}

fn unit_profile(ctx, transition) {
    if transition == "host" { rust_host_profile(ctx) } else { ctx.profile }
}

fn unit_profile_args(ctx, transition) {
    rust_crate_profile_args(unit_profile(ctx, transition), transition == "target");
}

fn dependency_search_args(ctx) {
    let directories = [];
    for dep in ctx.dep_artifacts {
        let parts = dep[1].split("/");
        parts.pop();
        let directory = "";
        for part in parts {
            if directory != "" { directory += "/"; }
            directory += part;
        }
        if !directories.contains(directory) { directories.push(directory); }
    }
    let args = [];
    for directory in directories {
        args += ["-L", `dependency=${directory}`];
    }
    args
}

fn dependency_key(src) {
    `${src.name}@${src.version}`
}

fn rust_lib_dir(ctx) {
    `forge-out/lib/${ctx.profile.name}`
}

fn rust_deps_dir(ctx) {
    `forge-out/lib/deps/${ctx.profile.name}`
}

fn rust_host_deps_dir(ctx) {
    `forge-out/lib/deps/host/${ctx.profile.name}`
}

fn unit_deps_dir(ctx, transition) {
    if transition == "host" { rust_host_deps_dir(ctx) } else { rust_deps_dir(ctx) }
}

fn configuration_artifact(artifacts, proc_macro, key, transition) {
    let own = artifacts[transition];
    if own != () { return own; }
    if transition == "target" && proc_macro[key] {
        let host = artifacts["host"];
        if host != () { return host; }
    }
    throw `dependency ${key} is not compiled for the ${transition} configuration`;
}

fn host_artifact(artifacts, key) {
    let path = artifacts["host"];
    if path == () { throw `dependency ${key} is not compiled for the host configuration`; }
    path
}

fn build_output_dir(ctx, name, version) {
    `forge-out/build/${ctx.profile.name}/${name}-${version}`
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
            if !rust_cfg_matches(ctx, key) { continue; }
            let section = targets[key];
            let targeted = section["dependencies"];
            if targeted != () { sections.push(targeted); }
            let targeted_build = section["build-dependencies"];
            if targeted_build != () { sections.push(targeted_build); }
        }
    }
    sections
}

fn active_dependency_sections(ctx, manifest) {
    let sections = [];
    let deps = manifest["dependencies"];
    if deps != () { sections.push(deps); }
    let targets = manifest["target"];
    if targets != () {
        for key in targets.keys() {
            if !rust_cfg_matches(ctx, key) { continue; }
            let targeted = targets[key]["dependencies"];
            if targeted != () { sections.push(targeted); }
        }
    }
    sections
}

fn active_dependency_names(ctx, manifest) {
    let names = [];
    for section in active_dependency_sections(ctx, manifest) { names += section_dep_names(section); }
    names
}

fn active_build_sections(ctx, manifest) {
    let sections = [];
    let build = manifest["build-dependencies"];
    if build != () { sections.push(build); }
    let targets = manifest["target"];
    if targets != () {
        for key in targets.keys() {
            if !rust_cfg_matches(ctx, key) { continue; }
            let targeted_build = targets[key]["build-dependencies"];
            if targeted_build != () { sections.push(targeted_build); }
        }
    }
    sections
}

fn active_build_names(ctx, manifest) {
    let names = [];
    for section in active_build_sections(ctx, manifest) { names += section_dep_names(section); }
    names
}

fn dependency_index(sections) {
    let index = #{};
    for section in sections {
        for dependency_name in section.keys() {
            let spec = section[dependency_name];
            let resolved = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { dependency_name };
            if !index.contains(resolved) { index[resolved] = #{ name: dependency_name, spec: spec }; }
        }
    }
    index
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

fn source_keys(by_name, package_names) {
    let found = [];
    for package_name in package_names {
        let key = by_name[package_name];
        if key != () && !found.contains(key) { found.push(key); }
    }
    found
}

fn unit_crate_name(entry) {
    let lib = entry["manifest"]["lib"];
    let name = if lib != () && lib["name"] != () { lib["name"] } else { entry["src"].name };
    rust_name(name)
}

fn unit_links(manifest) {
    let pkg = manifest["package"];
    if pkg == () { () } else { pkg["links"] }
}

fn unit_links_name(unit) {
    if unit["links"] == () { "" } else { unit["links"] }
}

fn proc_macro_keys(proc_macro, keys) {
    let found = [];
    for key in keys {
        if proc_macro[key] && !found.contains(key) { found.push(key); }
    }
    found
}

fn manifest_declares_dependencies(manifest) {
    if manifest["dependencies"] != () { return true; }
    let targets = manifest["target"];
    if targets == () { return false; }
    for key in targets.keys() {
        if targets[key]["dependencies"] != () || targets[key]["build-dependencies"] != () { return true; }
    }
    false
}

fn workspace_dependency_names(ctx, build) {
    if rust_metadata(ctx)["manifest"] != () {
        let manifest = workspace_crate_manifest(ctx);
        return if build { active_build_names(ctx, manifest) } else { active_dependency_names(ctx, manifest) };
    }
    if rust_mode(ctx["cell_config"]) == "native" {
        let own = workspace_crate_manifest(ctx);
        let names = if build { active_build_names(ctx, own) } else { active_dependency_names(ctx, own) };
        let cell_config = ctx["cell_config"];
        let catalog = if cell_config == () { () } else { cell_config["dependencies"] };
        if ctx.contains("targets") {
            for metadata in ctx.targets {
                let rust = metadata["rust"];
                if rust == () || rust.type_of() != "map" { continue; }
                if build {
                    if rust["build-dependencies"] != () { names += section_dep_names(rust["build-dependencies"]); }
                } else {
                    let deps = rust_inherited_dependencies(rust["dependencies"], catalog, "cell.rust.dependencies");
                    if deps != () { names += section_dep_names(deps); }
                }
            }
        }
        return names;
    }
    let names = [];
    for manifest in cargo_member_manifests(ctx) {
        names += if build { active_build_names(ctx, manifest) } else { active_dependency_names(ctx, manifest) };
    }
    names
}

fn unit_configurations(ctx, by_name, adjacency, normal_edges, build_edges, proc_macro, keys) {
    let host = graph_reachable(adjacency, source_keys(by_name, workspace_dependency_names(ctx, true)));
    let target_seeds = [];
    if rust_mode(ctx["cell_config"]) == "native" {
        for key in graph_roots(adjacency) {
            if !host.contains(key) { target_seeds.push(key); }
        }
    }
    for key in source_keys(by_name, workspace_dependency_names(ctx, false)) {
        if !target_seeds.contains(key) { target_seeds.push(key); }
    }
    let target = graph_reachable(normal_edges, target_seeds);
    let host_roots = [];
    for key in target {
        for dep_key in build_edges[key] {
            if !host.contains(dep_key) { host_roots.push(dep_key); }
        }
        if proc_macro[key] && !host.contains(key) { host_roots.push(key); }
    }
    for key in graph_reachable(adjacency, host_roots) {
        if !host.contains(key) { host.push(key); }
    }
    let configurations = #{};
    for key in keys { configurations[key] = []; }
    for key in host {
        configurations[key].push("host");
    }
    for key in target {
        if proc_macro[key] { continue; }
        configurations[key].push("target");
    }
    configurations
}
