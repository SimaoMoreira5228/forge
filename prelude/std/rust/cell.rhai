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
                name: "rustc dependency " + dependency_key(src),
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
        let out = ctx.lib_path(`lib${ctx.name}.rlib`);
        let args = ["--crate-type", "lib", "--crate-name", name, "--edition", edition, "-C", `metadata=${name}-${ctx.profile.name}`, "-L", "dependency=forge-out/lib", "--remap-path-prefix", "FORGE_EXEC_ROOT=", "-L", "dependency=forge-out/lib/deps"];
        args += dependency_search_args(ctx);
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
    args += dependency_search_args(ctx);
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
