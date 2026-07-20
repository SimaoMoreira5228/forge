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

fn build(ctx) {
    let rustc = rustc_for(ctx);
    let edition = edition_for(ctx);
    let name = crate_name(ctx);

    if ctx.kind == "library" {
        if ctx.srcs.len() == 0 {
            throw `library ${ctx.label} declares no sources`;
        }
        let out = rlib_path(ctx, ctx.name);
        let args = ["--crate-type", "lib", "--crate-name", name, "--edition", edition, "-o", out] + ctx.srcs;
        ctx.action(#{
            name: "rustc " + ctx.label + " -> " + out,
            command: rustc,
            args: args,
            inputs: ctx.srcs + ctx.hdrs + ctx.dep_archives,
            outputs: [out],
            env: ctx.env,
            toolchain_id: ctx.tool_id(),
        });
        return;
    }

    let out_dir = if ctx.kind == "test" { "forge-out/test" } else { "forge-out/bin" };
    let exe = `${out_dir}/${ctx.profile.name}/${ctx.name}`;
    let args = ["--crate-type", "bin", "--crate-name", name, "--edition", edition];
    for dep in ctx.dep_artifacts {
        args.push("--extern");
        args.push(`${dep[0]}=${dep[1]}`);
    }
    args += ["-o", exe] + ctx.srcs;
    let outputs = [exe];
    ctx.action(#{
        name: "rustc " + ctx.label + " -> " + outputs[0],
        command: rustc,
        args: args,
        inputs: ctx.srcs + ctx.hdrs + ctx.dep_archives,
        outputs: outputs,
        env: ctx.env,
        toolchain_id: ctx.tool_id(),
    });

    if ctx.kind == "test" {
        ctx.action(#{
            name: "run " + ctx.label,
            command: outputs[0],
            args: ctx.run_args,
            inputs: outputs + ctx.data,
            outputs: [],
            env: ctx.env,
            toolchain_id: "",
        });
    }
}
