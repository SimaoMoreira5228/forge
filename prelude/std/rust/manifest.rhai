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
