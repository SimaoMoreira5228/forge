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
    if rust_mode(ctx["cell_config"]) == "native" {
        let catalog = ctx["cell_config"]["dependencies"];
        for metadata in ctx.targets {
            let rust = metadata["rust"];
            if rust == () || rust.type_of() != "map" || rust["manifest"] != () { continue; }
            let deps = rust_inherited_dependencies(rust["dependencies"], catalog, "cell.rust.dependencies");
            if deps != () { requests = collect_requests(deps, requests); }
        }
        return requests;
    }
    if rust_metadata(ctx)["manifest"] != () {
        for section in active_dep_sections(ctx, workspace_crate_manifest(ctx)) {
            requests = collect_requests(section, requests);
        }
        return requests;
    }
    for manifest in cargo_member_manifests(ctx) {
        for section in active_dep_sections(ctx, manifest) { requests = collect_requests(section, requests); }
    }
    requests
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

fn closure_artifacts(plan, keys) {
    let artifacts = [];
    for key in keys { artifacts.push(host_artifact(plan.get("artifacts", key), key)); }
    artifacts
}

fn configuration_artifacts(plan, keys, transition) {
    let artifacts = [];
    let proc_macro = plan.get("proc_macro");
    for key in keys {
        artifacts.push(configuration_artifact(plan.get("artifacts", key), proc_macro, key, transition));
    }
    artifacts
}

fn host_closure_artifacts(plan, keys) {
    let artifacts = [];
    for key in keys {
        for reached in plan.get("graph", key)["reachable"] {
            let artifact = host_artifact(plan.get("artifacts", reached), reached);
            if !artifacts.contains(artifact) { artifacts.push(artifact); }
        }
    }
    artifacts
}

fn locked_dependency(plan, parent, section, name) {
    let spec = section[name];
    let package_name = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { name };
    let key = plan.get("by_name", package_name);
    if key == () { return (); }
    if parent != () && !plan.get("graph", parent)["full"].contains(key) { return (); }
    #{ name: name, unit: plan.get("units", key) }
}

fn build_dependency_externs(ctx, plan, parent, sections) {
    let externs = [];
    for section in sections {
        for name in section.keys() {
            let dep = locked_dependency(plan, parent, section, name);
            if dep == () { continue; }
            let unit = dep["unit"];
            let extern_name = name;
            extern_name.replace("-", "_");
            externs.push(#{
                name: extern_name,
                path: host_artifact(plan.get("artifacts", unit["key"]), unit["key"]),
                closure: closure_artifacts(plan, plan.get("graph", unit["key"])["host"]),
                links: unit_links_name(unit),
                dir: build_output_dir(ctx, unit["name"], unit["version"]),
            });
        }
    }
    externs
}

fn linked_dependencies(ctx, plan, parent, sections) {
    let deps = [];
    for section in sections {
        for name in section.keys() {
            let dep = locked_dependency(plan, parent, section, name);
            if dep == () { continue; }
            let unit = dep["unit"];
            let links = unit_links_name(unit);
            if links != "" {
                let dir = build_output_dir(ctx, unit["name"], unit["version"]);
                if !deps.contains(#{ name: name, links: links, dir: dir }) { deps.push(#{ name: name, links: links, dir: dir }); }
            }
        }
    }
    deps
}
