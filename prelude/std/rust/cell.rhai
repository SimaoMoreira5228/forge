fn plan(ctx) {
    let sources = ctx.fetched_sources;
    let meta = #{};
    let keys = [];
    let by_name = #{};
    let first_key = #{};
    let proc_macro = #{};
    let build_sections = #{};
    let dep_sections = #{};
    for src in sources {
        let key = dependency_key(src);
        let manifest = if ctx.glob(`${src.root}/Cargo.toml`).len() == 0 {
            #{ "package": #{}, "lib": #{}, "features": #{} }
        } else {
            toml_decode(ctx.read_file(`${src.root}/Cargo.toml`))
        };
        let lib = manifest["lib"];
        let entry = #{
            src: src,
            manifest: manifest,
            edition: edition_of(manifest),
            proc_macro: lib != () && lib["proc-macro"] == true,
            name: src.name,
        };
        meta[key] = entry;
        by_name[src.name] = key;
        if !first_key.contains(src.name) { first_key[src.name] = key; }
        proc_macro[key] = entry["proc_macro"];
        build_sections[key] = active_build_sections(ctx, manifest);
        dep_sections[key] = active_dep_sections(ctx, manifest);
        keys.push(key);
    }

    let adjacency = #{};
    let normal_edges = #{};
    let build_edges = #{};
    for key in keys {
        let src = meta[key]["src"];
        let declared = manifest_declares_dependencies(meta[key]["manifest"]);
        let normal_names = active_dependency_names(ctx, meta[key]["manifest"]);
        let build_names = active_build_names(ctx, meta[key]["manifest"]);
        let deps = [];
        let normal = [];
        let build = [];
        for dep_str in src.dependencies {
            let dep_name = dep_str.split(" ")[0];
            let dep_version = if dep_str.contains(" ") { dep_str.split(" ")[1] } else { "" };
            let package_name = if dep_str.contains(" package ") { dep_str.split(" package ")[1].split(" ")[0] } else { dep_name };
            let pinned = `${package_name}@${dep_version}`;
            let dep_key = if dep_version == "" { first_key[package_name] } else if meta.contains(pinned) { pinned } else { () };
            if dep_key == () { continue; }
            if !declared || normal_names.contains(package_name) { normal.push(dep_key); }
            if !declared || build_names.contains(package_name) { build.push(dep_key); }
            if normal.contains(dep_key) || build.contains(dep_key) { deps.push(dep_key); }
        }
        adjacency[key] = deps;
        normal_edges[key] = normal;
        build_edges[key] = build;
    }

    let configurations = unit_configurations(ctx, by_name, adjacency, normal_edges, build_edges, proc_macro, keys);


    let artifacts = #{};
    for key in keys {
        let entry = meta[key];
        let crate = unit_crate_name(entry);
        let suffix = if entry["proc_macro"] { proc_macro_dylib_suffix(ctx) } else { "rlib" };
        let own = #{};
        for transition in configurations[key] {
            own[transition] = `${unit_deps_dir(ctx, transition)}/lib${crate}-${entry["src"].version}.${suffix}`;
        }
        artifacts[key] = own;
    }

    let units = #{};
    let package_files = #{};
    let graphs = #{};
    let forms = #{};
    let edge_sets = #{};
    let feature_tables = #{};
    let declared_features = #{};
    let source_names = #{};
    let dep_defaults = #{};
    let owned = #{};
    for key in keys {
        let entry = meta[key];
        let src = entry["src"];
        let manifest = entry["manifest"];
        let feature_table = manifest["features"];
        let defaults = defaults_of(manifest);
        let own_artifacts = artifacts[key];
        let files = ctx.glob(`${src.root}/**/*`);
        let closure = graph_transitive(normal_edges, key);
        let host_closure = [key];
        for dep_key in graph_transitive(adjacency, key) { host_closure.push(dep_key); }
        let host_reachable = graph_reachable(adjacency, [key]);
        let host_deps = adjacency[key];
        let target_deps = normal_edges[key];
        let host_transitive = graph_transitive(adjacency, key);
        let target_transitive = closure;
        let dep_index = dependency_index(dep_sections[key]);
        let edges = [];
        let crates = #{};
        for dep_key in host_deps {
            let dep_name = source_names[dep_key];
            let entry_for_dep = dep_index[dep_name];
            if entry_for_dep == () { continue; }
            let dep_crate = rust_name(entry_for_dep["name"]);
            crates[dep_key] = dep_crate;
            edges.push(#{
                dep: dep_key,
                spec: entry_for_dep["spec"],
                dep_defaults: dep_defaults[dep_key],
                prefixes: [`${entry_for_dep["name"]}/`, `${dep_name}/`, `${entry_for_dep["name"]}?/`, `${dep_name}?/`],
            });
        }
        let lib = manifest["lib"];
        let crate_path = if lib != () && lib["path"] != () { lib["path"] } else { "src/lib.rs" };
        let crate_name_of_unit = unit_crate_name(entry);
        let version = src.version;
        let unit_forms = #{};
        for transition in configurations[key] {
            let deps = if transition == "host" { host_deps } else { target_deps };
            let transitive = if transition == "host" { host_transitive } else { target_transitive };
            let macros = if transition == "host" { [] } else { proc_macro_keys(proc_macro, transitive) };
            let crate_type = if entry["proc_macro"] { "proc-macro" } else { "lib" };
            let head = ["--crate-type", crate_type, "--crate-name", crate_name_of_unit, "--edition", entry["edition"], "-C", `metadata=${unit_metadata(crate_name_of_unit, version, transition)}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", `dependency=${unit_deps_dir(ctx, transition)}`];
            if crate_type == "proc-macro" { head += ["--extern", "proc_macro"]; }
            let search = if transition == "host" || macros.len() == 0 {
                []
            } else {
                ["-L", `dependency=${rust_host_deps_dir(ctx)}`]
            };
            let externs = [];
            for dep_key in deps {
                externs += ["--extern", `${crates[dep_key]}=${configuration_artifact(artifacts[dep_key], proc_macro, dep_key, transition)}`];
            }
            let env = cargo_pkg_env(manifest, src.name, version);
            env["CARGO_CRATE_NAME"] = crate_name_of_unit;
            env["CARGO_MANIFEST_DIR"] = `FORGE_EXEC_ROOT/${src.root}`;
            env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${src.root}/Cargo.toml`;
            unit_forms[transition] = #{
                name: `rustc dependency ${key} (${transition})`,
                configuration: transition,
                head: head,
                profile: unit_profile_args(ctx, transition),
                search: search,
                externs: externs,
                closure: transitive,
                macros: macros,
                outputs: [own_artifacts[transition]],
                env: env,
                crate_root: `${src.root}/${crate_path}`,
            };
        }
        units[key] = #{
            key: key,
            name: src.name,
            version: version,
            root: src.root,
            edition: entry["edition"],
            proc_macro: entry["proc_macro"],
            links: unit_links(manifest),
            configurations: configurations[key],
        };
        edge_sets[key] = edges;
        feature_tables[key] = feature_table;
        declared_features[key] = defaults;
        dep_defaults[key] = defaults;
        source_names[key] = src.name;
        graphs[key] = #{
            normal: closure,
            full: host_deps,
            host: host_closure,
            reachable: host_reachable,
        };
        forms[key] = unit_forms;
        package_files[key] = files;
        owned[key] = own_artifacts;
    }

    let enabled = feature_closure(graph_topo(adjacency), workspace_root_requests(ctx), source_names, edge_sets, declared_features, feature_tables);
    let shapes = #{};
    for key in keys {
        let entry = meta[key];
        shapes[key] = build_script_shape(entry["manifest"], entry["name"], entry["src"].version, entry["src"].root, enabled[key]);
    }
    #{
        order: keys,
        by_name: by_name,
        proc_macro: proc_macro,
        artifacts: owned,
        package_files: package_files,
        enabled: enabled,
        graph: graphs,
        forms: forms,
        units: units,
        shapes: shapes,
        build_sections: build_sections,
        dep_sections: dep_sections,
    }
}

fn feature_closure(order, requests, names, edges, declared, feature_tables) {
    let enabled = #{};
    for key in order {
        enabled[key] = [];
        if requests.contains(names[key]) {
            let request = requests[names[key]];
            if request["defaults"] {
                enabled[key] = append_features(enabled[key], declared[key]);
            }
            enabled[key] = append_features(enabled[key], request["features"]);
        }
    }
    for key in order {
        enabled[key] = expand_features(feature_tables[key], enabled[key]);
        for edge in edges[key] {
            let dep_key = edge["dep"];
            let spec = edge["spec"];
            if spec.type_of() != "map" || spec["default-features"] != false {
                enabled[dep_key] = append_features(enabled[dep_key], edge["dep_defaults"]);
            }
            if spec.type_of() == "map" && spec["features"] != () {
                enabled[dep_key] = append_features(enabled[dep_key], spec["features"]);
            }
            let parent_features = feature_tables[key];
            if parent_features != () {
                for parent_feature in parent_features.keys() {
                    if !enabled[key].contains(parent_feature) { continue; }
                    for member in parent_features[parent_feature] {
                        for prefix in edge["prefixes"] {
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

fn defaults_of(manifest) {
    let table = manifest["features"];
    if table == () || table["default"] == () { [] } else { table["default"] }
}

fn unit_metadata(crate_name, version, transition) {
    if transition == "host" { `${crate_name}-${version}-host` } else { `${crate_name}-${version}` }
}

fn build(ctx, plan) {
    let rustc = rustc_for(ctx);
    let edition = edition_for(ctx);
    let name = crate_name(ctx);
    let order = plan.get("order");
    let enabled = plan.get("enabled");

    if order.len() > 0 && plan.once("fetched sources") {
        for key in order {
            let unit = plan.get("units", key);
            if unit["configurations"].len() == 0 { continue; }
            let build_externs = build_dependency_externs(ctx, plan, key, plan.get("build_sections", key));
            let build_closure = [];
            for dep in build_externs { build_closure += dep["closure"]; }
            let build_output = emit_build_script(ctx, rustc, merge_env(plan.get("shapes", key), #{
                features: enabled[key],
                root: unit["root"],
                name: unit["name"],
                version: unit["version"],
                edition: unit["edition"],
                build_externs: build_externs,
                build_link_deps: linked_dependencies(ctx, plan, key, plan.get("dep_sections", key)),
                package_files: plan.get("package_files", key),
                build_closure: build_closure,
            }));
            for transition in unit["configurations"] {
                emit_dependency_unit(ctx, plan, unit, transition, rustc, enabled[key], build_output);
            }
        }
    }

    let crate_manifest = workspace_crate_manifest(ctx);
    let direct = active_dependency_names(ctx, crate_manifest);
    let extern_keys = [];
    for key in order {
        let unit = plan.get("units", key);
        if !direct.contains(unit["name"]) { continue; }
        if !extern_keys.contains(key) { extern_keys.push(key); }
        for reached in plan.get("graph", key)["normal"] {
            if !extern_keys.contains(reached) { extern_keys.push(reached); }
        }
    }
    let external_inputs = [];
    let external_externs = [];
    let external_names = [];
    for key in order {
        let unit = plan.get("units", key);
        if !extern_keys.contains(key) || !unit["configurations"].contains("target") { continue; }
        let out = plan.get("artifacts", key)["target"];
        external_inputs.push(out);
        if !direct.contains(unit["name"]) { continue; }
        let crate_n = dependency_crate_name(ctx, crate_manifest, unit["name"], crate_name(#{ name: unit["name"] }));
        if external_names.contains(crate_n) { throw `multiple locked versions for direct dependency ${crate_n}; select explicitly in a future importer milestone`; }
        external_names.push(crate_n);
        external_externs.push(`${crate_n}=${out}`);
    }
    let proc_macros = proc_macro_keys(plan.get("proc_macro"), extern_keys);
    for artifact in host_closure_artifacts(plan, proc_macros) { external_inputs.push(artifact); }

    let crate_env = workspace_crate_env(ctx);
    let crate_root_dir = workspace_crate_root(ctx);
    crate_manifest["forge-manifest-path"] = workspace_manifest_path(ctx);
    let crate_version = if crate_manifest["package"] != () && crate_manifest["package"]["version"].type_of() == "string" { crate_manifest["package"]["version"] } else { workspace_package_version(ctx) };
    let crate_entry = if ctx.kind == "library" { "src/lib.rs" } else { "src/main.rs" };
    let declared_root = rust_metadata(ctx)["crate_root"];
    let conventional_root = rust_path(crate_root_dir, crate_entry);
    let crate_root_file = if declared_root != () {
        rust_path("", declared_root)
    } else if ctx.srcs.contains(conventional_root) {
        conventional_root
    } else if ctx.srcs.len() > 0 {
        ctx.srcs[0]
    } else {
        throw `component ${ctx.label} declares no crate root`;
    };
    let crate_features = expand_features(crate_manifest["features"], if rust_metadata(ctx)["enabled_features"] == () {
        if crate_manifest["features"] == () || crate_manifest["features"]["default"] == () { [] } else { crate_manifest["features"]["default"] }
    } else { rust_metadata(ctx)["enabled_features"] });
    let package_files = ctx.srcs + ctx.hdrs;
    let build_deps = build_dependency_externs(ctx, plan, (), active_build_sections(ctx, crate_manifest));
    let build_script_files = ctx.hdrs;
    for dep in build_deps { build_script_files += dep["closure"]; }
    let build_output = emit_build_script(ctx, rustc, merge_env(build_script_shape(crate_manifest, ctx.name, crate_version, crate_root_dir, crate_features), #{
        features: crate_features,
        root: crate_root_dir,
        name: ctx.name,
        version: crate_version,
        edition: edition,
        build_externs: build_deps,
        build_link_deps: linked_dependencies(ctx, plan, (), active_dep_sections(ctx, crate_manifest)),
        package_files: package_files,
        build_closure: build_script_files,
    }));
    let crate_env = if build_output != "" { merge_env(crate_env, #{ "OUT_DIR": `FORGE_EXEC_ROOT/${build_output}` }) } else { crate_env };
    let dep_inputs = ctx.srcs + ctx.hdrs;
    if build_output != "" { dep_inputs.push(build_output); }
    let execution_deps = ctx.dep_archives + external_inputs;
    for dep in ctx.dep_artifacts {
        execution_deps.push(dep[1]);
    }
    if build_output != "" { execution_deps.push(build_output); }
    let link_args = [];
    for key in order {
        let unit = plan.get("units", key);
        if unit["configurations"].len() == 0 { continue; }
        if unit["links"] == () { continue; }
        let output_file = `${build_output_dir(ctx, unit["name"], unit["version"])}/.forge-build-output`;
        for directive in ["link-lib", "link-search"] {
            let flag = if directive == "link-lib" { "-l" } else { "-L" };
            link_args.push(#{ path: output_file, prefix: `cargo:rustc-${directive}=`, flag: flag, root_marker: "forge-out/" });
            link_args.push(#{ path: output_file, prefix: `cargo::rustc-${directive}=`, flag: flag, root_marker: "forge-out/" });
        }
        execution_deps.push(build_output_dir(ctx, unit["name"], unit["version"]));
    }
    let crate_argument_files = if build_output != "" { link_args + build_output_argument_files(build_output) } else { link_args };

    if ctx.kind == "library" {
        if ctx.srcs.len() == 0 {
            throw `library ${ctx.label} declares no sources`;
        }
        let out = ctx.lib_path(`lib${ctx.name}.rlib`);
        let args = ["--crate-type", "lib", "--crate-name", name, "--edition", edition, "-C", `metadata=${name}-${ctx.profile.name}`, "-L", `dependency=${rust_lib_dir(ctx)}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", `dependency=${rust_deps_dir(ctx)}`];
        if proc_macros.len() > 0 { args += ["-L", `dependency=${rust_host_deps_dir(ctx)}`]; }
        args += dependency_search_args(ctx) + feature_cfg_args(crate_manifest, crate_features) + rust_profile_args(ctx) + ctx.flags;
        for dep in ctx.dep_artifacts {
            args.push("--extern");
            args.push(`${workspace_extern_name(ctx, dep[0])}=${dep[1]}`);
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
            artifact: out,
            argument_files: crate_argument_files,
            env: crate_env,
            toolchain_id: ctx.tool_id(),
        });
        return;
    }

    let out_dir = if ctx.kind == "test" { "forge-out/test" } else { "forge-out/bin" };
    let exe = `${out_dir}/${ctx.profile.name}/${ctx.name}`;
    let args = ["--crate-type", "bin", "--crate-name", name, "--edition", edition, "-C", `metadata=${name}-${ctx.profile.name}`, "-L", `dependency=${rust_lib_dir(ctx)}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", `dependency=${rust_deps_dir(ctx)}`];
    if proc_macros.len() > 0 { args += ["-L", `dependency=${rust_host_deps_dir(ctx)}`]; }
    if ctx.kind == "test" { args.push("--test"); }
    args += dependency_search_args(ctx) + feature_cfg_args(crate_manifest, crate_features) + rust_profile_args(ctx) + ctx.flags;
    for dep in ctx.dep_artifacts {
        args.push("--extern");
        args.push(`${workspace_extern_name(ctx, dep[0])}=${dep[1]}`);
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
            inputs: ctx.data,
            execution_deps: outputs,
            outputs: [],
            is_test: true,
            env: crate_env,
            toolchain_id: "",
        });
    }
}

fn emit_dependency_unit(ctx, plan, unit, transition, rustc, features, build_output) {
    let form = plan.get("forms", unit["key"])[transition];
    let closure_artifacts = configuration_artifacts(plan, form["closure"], transition);
    let macro_artifacts = host_closure_artifacts(plan, form["macros"]);
    let env = merge_env(string_env(ctx.env), form["env"]);
    let args = form["head"] + plan.get("shapes", unit["key"])["cfg"] + form["profile"] + form["search"] + form["externs"];
    let argument_files = [];
    if build_output != "" {
        env["OUT_DIR"] = `FORGE_EXEC_ROOT/${build_output_dir(ctx, unit["name"], unit["version"])}`;
        env["CRUNCHY_LIB_SUFFIX"] = "/lib.rs";
        argument_files += build_output_argument_files(build_output);
    }
    ctx.action(#{
        name: form["name"],
        configuration: transition,
        command: rustc,
        args: args + ["-o", form["outputs"][0], form["crate_root"]],
        inputs: plan.get("package_files", unit["key"]) + closure_artifacts + macro_artifacts,
        execution_deps: closure_artifacts + macro_artifacts + (if build_output == "" { [] } else { [build_output] }),
        outputs: form["outputs"],
        argument_files: argument_files,
        env: env,
        toolchain_id: ctx.tool_id(),
    });
}