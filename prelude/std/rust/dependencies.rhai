fn rust_catalog_entry(catalog, source, name) {
    if catalog == () { return (); }
    if catalog.type_of() != "map" { throw `${source} must be a table`; }
    let entry = catalog[name];
    if entry == () { return (); }
    if entry.type_of() == "string" { return #{ version: entry }; }
    if entry.type_of() != "map" { throw `inherited dependency ${name} in ${source} must be a version string or a table`; }
    entry
}

fn rust_inherited_dependency(spec, catalog, source, name) {
    if spec.type_of() != "map" || spec["workspace"] != true { return spec; }
    let inherited = rust_catalog_entry(catalog, source, name);
    if inherited == () { throw `dependency ${name} inherits ${source}, which does not declare ${name}`; }
    for key in spec.keys() {
        if !["workspace", "features", "default-features"].contains(key) {
            throw `dependency ${name} inherits ${source} and also sets ${key}; an inherited declaration may only add features and default-features`;
        }
    }
    let merged = #{};
    for key in inherited.keys() { merged[key] = inherited[key]; }
    if inherited["features"] != () || spec["features"] != () {
        let features = [];
        if inherited["features"] != () { features += inherited["features"]; }
        if spec["features"] != () { features += spec["features"]; }
        merged["features"] = features;
    }
    if spec["default-features"] != () { merged["default-features"] = spec["default-features"]; }
    merged
}

fn rust_inherited_dependencies(declarations, catalog, source) {
    if declarations == () { return (); }
    if declarations.type_of() != "map" { throw "Rust dependencies must be a table"; }
    let merged = #{};
    for name in declarations.keys() { merged[name] = rust_inherited_dependency(declarations[name], catalog, source, name); }
    merged
}

fn cargo_inherit(manifest, workspace) {
    let pkg = manifest["package"];
    if pkg == () { throw "Cargo package manifest requires [package]"; }
    for key in pkg.keys() {
        if key == "metadata" || pkg[key].type_of() != "map" { continue; }
        if pkg[key]["workspace"] != true || workspace["package"] == () || workspace["package"][key] == () {
            throw `unsupported or missing workspace.package.${key} inheritance`;
        }
        pkg[key] = workspace["package"][key];
    }
    manifest["package"] = pkg;
    for section_name in ["dependencies", "build-dependencies", "dev-dependencies"] {
        let section = manifest[section_name];
        if section == () { continue; }
        for name in section.keys() {
            let spec = section[name];
            if spec.type_of() != "map" || spec["workspace"] != true { continue; }
            let inherited = rust_inherited_dependency(spec, workspace["dependencies"], "workspace.dependencies", name);
            if inherited["path"] != () { inherited["forge-workspace-path"] = true; }
            section[name] = inherited;
        }
        manifest[section_name] = section;
    }
    manifest
}

fn rust_resolve(targets, catalog, selected, conflict) {
    let pending = [];
    for metadata in targets {
        let rust = metadata["rust"];
        if rust == () || rust.type_of() != "map" { continue; }
        if rust["manifest"] != () { continue; }
        pending += rust_resolve_section(rust_inherited_dependencies(rust["dependencies"], catalog, "cell.rust.dependencies"));
        pending += rust_resolve_section(rust_inherited_dependencies(rust["build-dependencies"], catalog, "cell.rust.dependencies"));
    }
    if pending.len() > 0 && resolving_dependencies {
        rust_registry_candidates(pending, selected, conflict);
    }
}

fn rust_resolve_section(declarations) {
    let pending = [];
    if declarations == () { return pending; }
    if declarations.type_of() != "map" { throw "Rust dependencies must be a table"; }
    for alias in declarations.keys() {
        let spec = declarations[alias];
        if spec.type_of() != "map" { throw `Rust dependency ${alias} must be a table`; }
        for key in spec.keys() {
            if !["name", "version", "source", "checksum", "dependencies", "range", "registry", "features", "default-features", "git", "rev"].contains(key) { throw `unknown Rust dependency key: ${key}`; }
        }
        let name = if spec["name"] == () { alias } else { spec["name"] };
        if spec.contains("range") {
            let registry = rust_registry_index_url(spec.registry);
            try {
                for key in ["version", "source", "checksum", "dependencies"] {
                    if spec.contains(key) { throw `range cannot be combined with ${key}`; }
                }
                rust_name(name);
                name = name.to_lower();
                if spec["features"] != () && spec["features"].type_of() != "array" { throw "dependency features must be an array"; }
                let range = rust_registry_range(spec.range);
                dependency_require(name, range.min, range.max);
                pending.push(#{
                    name: name,
                    min: range.min,
                    max: range.max,
                    registry: registry,
                    features: if spec["features"] == () { [] } else { spec["features"] },
                    "default": spec["default-features"] != false,
                });
            } catch (error) { throw `Rust registry ${registry}, dependency ${alias}: ${error}`; }
        } else if spec.contains("git") {
            if spec["version"] == () || spec["rev"] == () { throw `git dependency ${alias} requires version and rev`; }
            let deps = if spec["dependencies"] == () { [] } else { spec["dependencies"] };
            dependency_git(name, spec["version"], spec["git"], spec["rev"], deps);
        } else {
            if spec.contains("registry") { throw `native registry requires range: ${spec.registry}`; }
            let deps = if spec["dependencies"] == () { [] } else { spec["dependencies"] };
            dependency(name, spec["version"], spec["source"], spec["checksum"], deps);
        }
    }
    pending
}

if is_def_var("targets") {
    rust_resolve(targets, cell_config["dependencies"], selected_packages, conflict);
} else if is_def_var("cell_config") {
    rust_workspace(cell_config);
}
