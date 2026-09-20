fn build(ctx) {
    let rustc = rustc_for(ctx);
    let edition = edition_for(ctx);
    let name = crate_name(ctx);

    let info = source_meta(ctx);
    let meta = info["meta"];
    let adjacency = info["adjacency"];
    let features = feature_sets(ctx, meta, adjacency, graph_topo(adjacency), workspace_root_requests(ctx));

    if ctx.fetch_owner {
        for src in ctx.fetched_sources {
            let key = dependency_key(src);
            let configurations = meta[key]["configurations"];
            if configurations.len() == 0 { continue; }
            let build_output = emit_dependency_build_script(ctx, meta, info, key, rustc, features);
            for transition in configurations {
                emit_dependency_unit(ctx, meta, info, key, transition, rustc, features, build_output);
            }
        }
    }

    let crate_manifest = workspace_crate_manifest(ctx);
    let direct = active_dependency_names(ctx, crate_manifest);
    let extern_keys = [];
    for src in ctx.fetched_sources {
        let key = dependency_key(src);
        if !direct.contains(src.name) { continue; }
        if !extern_keys.contains(key) { extern_keys.push(key); }
        for reached in graph_transitive(info["normal_edges"], key) {
            if !extern_keys.contains(reached) { extern_keys.push(reached); }
        }
    }
    let external_inputs = [];
    let external_externs = [];
    let external_names = [];
    for src in ctx.fetched_sources {
        let key = dependency_key(src);
        if !extern_keys.contains(key) || !meta[key]["configurations"].contains("target") { continue; }
        let out = dependency_artifact(meta, key, "target");
        external_inputs.push(out);
        if !direct.contains(src.name) { continue; }
        let crate_n = dependency_crate_name(ctx, crate_manifest, src.name, crate_name(#{ name: src.name }));
        if external_names.contains(crate_n) { throw `multiple locked versions for direct dependency ${crate_n}; select explicitly in a future importer milestone`; }
        external_names.push(crate_n);
        external_externs.push(`${crate_n}=${out}`);
    }
    let proc_macros = proc_macro_keys(meta, extern_keys);
    let host_macros = proc_macro_host_artifacts(meta, info, proc_macros);
    for artifact in host_macros { external_inputs.push(artifact); }

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
    let build_deps = build_dependency_externs(ctx, meta, info, (), crate_manifest);
    let build_script_files = ctx.hdrs;
    for dep in build_deps { build_script_files += dep["closure"]; }
    let build_output = emit_build_script(ctx, rustc, crate_manifest, crate_features, crate_root_dir, ctx.name, crate_version, edition, build_deps, linked_dependencies(ctx, meta, info, (), crate_manifest), package_files, build_script_files);
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
        if meta[key]["configurations"].len() == 0 { continue; }
        let pkg = meta[key]["manifest"]["package"];
        if pkg == () || pkg["links"] == () { continue; }
        let dep_build_output = build_output_dir(ctx, src.name, src.version);
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
            env: crate_env,
            toolchain_id: "",
        });
    }
}

fn emit_dependency_build_script(ctx, meta, info, key, rustc, features) {
    let src = meta[key]["src"];
    let deps = build_dependency_externs(ctx, meta, info, key, meta[key]["manifest"]);
    let closure = [];
    for dep in deps { closure += dep["closure"]; }
    emit_build_script(
        ctx, rustc, meta[key]["manifest"], features[key], src.root, src.name, src.version, meta[key]["edition"],
        deps,
        linked_dependencies(ctx, meta, info, key, meta[key]["manifest"]),
        ctx.glob(`${src.root}/**/*`),
        closure,
    )
}

fn emit_dependency_unit(ctx, meta, info, key, transition, rustc, features, build_output) {
    let src = meta[key]["src"];
    let manifest = meta[key]["manifest"];
    let enabled = features[key];
    let lib = manifest["lib"];
    let pkg_name = unit_crate_name(meta[key]);
    let edition = meta[key]["edition"];
    let crate_type = if meta[key]["proc_macro"] { "proc-macro" } else { "lib" };
    let crate_path = if lib != () && lib["path"] != () { lib["path"] } else { "src/lib.rs" };
    let out = dependency_artifact(meta, key, transition);
    let unit_metadata = if transition == "host" { `${pkg_name}-${src.version}-host` } else { `${pkg_name}-${src.version}` };
    let args = ["--crate-type", crate_type, "--crate-name", pkg_name, "--edition", edition, "-C", `metadata=${unit_metadata}`, "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", `dependency=${unit_deps_dir(ctx, transition)}`];
    if crate_type == "proc-macro" { args += ["--extern", "proc_macro"]; }
    args += feature_cfg_args(manifest, enabled) + unit_profile_args(ctx, transition);
    let package_files = ctx.glob(`${src.root}/**/*`);
    let unit_graph = unit_adjacency(info, transition);
    let direct = unit_graph[key];
    let transitive = graph_transitive(unit_graph, key);
    let macros = if transition == "host" { [] } else { proc_macro_keys(meta, transitive) };
    if macros.len() > 0 { args += ["-L", `dependency=${rust_host_deps_dir(ctx)}`]; }
    let inputs = package_files + dependency_artifacts(meta, transitive, transition) + proc_macro_host_artifacts(meta, info, macros);
    let execution_deps = dependency_artifacts(meta, transitive, transition) + proc_macro_host_artifacts(meta, info, macros);
    if build_output != "" { execution_deps.push(build_output); }
    for dep_key in direct {
        let dep_src = meta[dep_key]["src"];
        let dep_out = dependency_artifact(meta, dep_key, transition);
        let dep_crate = dependency_crate_name(ctx, manifest, dep_src.name, crate_name(#{ name: dep_src.name }));
        args += ["--extern", `${dep_crate}=${dep_out}`];
    }
    let dep_env = merge_env(string_env(ctx.env), cargo_pkg_env(manifest, src.name, src.version));
    dep_env["CARGO_CRATE_NAME"] = pkg_name;
    dep_env["CARGO_MANIFEST_DIR"] = `FORGE_EXEC_ROOT/${src.root}`;
    dep_env["CARGO_MANIFEST_PATH"] = `FORGE_EXEC_ROOT/${src.root}/Cargo.toml`;
    let argument_files = [];
    if build_output != "" {
        dep_env["OUT_DIR"] = `FORGE_EXEC_ROOT/${build_output_dir(ctx, src.name, src.version)}`;
        dep_env["CRUNCHY_LIB_SUFFIX"] = "/lib.rs";
        argument_files += build_output_argument_files(build_output);
    }
    ctx.action(#{
        name: `rustc dependency ${key} (${transition})`,
        configuration: transition,
        command: rustc,
        args: args + ["-o", out, `${src.root}/${crate_path}`],
        inputs: inputs,
        execution_deps: execution_deps,
        outputs: [out],
        argument_files: argument_files,
        env: dep_env,
        toolchain_id: ctx.tool_id(),
    });
}
