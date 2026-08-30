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
        for metadata in ctx.targets {
            let rust = metadata["rust"];
            if rust == () || rust.type_of() != "map" || rust["manifest"] != () { continue; }
            let deps = rust["dependencies"];
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
    let root = toml_decode(ctx.read_file("Cargo.toml"));
    let workspace = if root["workspace"] == () { #{} } else { root["workspace"] };
    let paths = if root["package"] == () { [] } else { ["Cargo.toml"] };
    let excluded = [];
    if workspace["exclude"] != () {
        for member in workspace["exclude"] { excluded += ctx.glob(rust_path(member, "Cargo.toml")); }
    }
    if workspace["members"] != () {
        for member in workspace["members"] { paths += ctx.glob(rust_path(member, "Cargo.toml")); }
    }
    for path in paths {
        if excluded.contains(path) { continue; }
        let manifest = cargo_inherit(toml_decode(ctx.read_file(path)), workspace);
        for section in active_dep_sections(ctx, manifest) { requests = collect_requests(section, requests); }
    }
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

fn build_dependency_externs(ctx, meta, manifest) {
    let externs = [];
    for section in active_build_sections(ctx, manifest) {
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
    }
    externs
}

fn linked_dependencies(ctx, meta, manifest) {
    let deps = [];
    for section in active_dep_sections(ctx, manifest) {
        for name in section.keys() {
            let spec = section[name];
            let package_name = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { name };
            for key in meta.keys() {
                let src = meta[key]["src"];
                if meta[key]["active"] == false || src.name != package_name { continue; }
                let dep_manifest = meta[key]["manifest"];
                let links = if dep_manifest["package"] == () || dep_manifest["package"]["links"] == () { "" } else { dep_manifest["package"]["links"] };
                if links != "" {
                    deps.push(#{
                        name: name,
                        links: links,
                        dir: `forge-out/build/${src.name}-${src.version}`,
                    });
                }
                break;
            }
        }
    }
    deps
}
