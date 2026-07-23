fn is_cpp(src) {
    src.ends_with(".cpp") || src.ends_with(".cc") || src.ends_with(".cxx") || src.ends_with(".C");
}

fn compiler_for(ctx, src) {
    if ctx.compiler != "" {
        let selected = ctx.bin(ctx.compiler);
        if selected != "" {
            return selected;
        }
    }
    let candidates = if ctx.profile.coverage {
        if is_cpp(src) { ["g++", "clang++", "c++"] } else { ["gcc", "clang", "cc"] }
    } else if is_cpp(src) {
        ["clang++", "g++", "c++"]
    } else {
        ["clang", "gcc", "cc"]
    };
    for name in candidates {
        let path = ctx.bin(name);
        if path != "" {
            return path;
        }
    }
    throw `toolchain provides none of the compilers for ${src}`;
}

fn ar_for(ctx) {
    let path = ctx.bin("ar");
    if path == "" {
        throw `toolchain provides no ar`;
    }
    path;
}

fn linker_for(ctx) {
    if ctx.linker != "" {
        return ctx.linker;
    }
    if ctx.compiler != "" {
        let selected = ctx.bin(ctx.compiler);
        if selected != "" {
            return selected;
        }
    }
    let candidates = if ctx.profile.coverage {
        ["g++", "gcc", "clang++", "clang", "c++", "cc"]
    } else {
        ["clang++", "g++", "c++", "clang", "gcc", "cc"]
    };
    for name in candidates {
        let found = ctx.bin(name);
        if found != "" {
            return found;
        }
    }
    throw `toolchain provides no linker`;
}

fn profile_args(c) {
    let args = [`-O${c.profile.opt_level}`];
    if c.profile.is_debug {
        args.push("-g");
    }
    for san in c.profile.sanitizers {
        args.push(`-fsanitize=${san}`);
    }
    if c.profile.coverage {
        args.push("--coverage");
    }
    if c.profile.lto {
        args.push("-flto");
    }
    for define in c.profile.defines {
        args.push(`-D${define}`);
    }
    args;
}

fn compile_one(ctx, src) {
    let default_std = if is_cpp(src) { "c++20" } else { "c17" };
    let chosen = if ctx.standard == "" { default_std } else { ctx.standard };

    let obj = ctx.obj_path(src);
    let depfile = `${obj}.d`;

    let args = profile_args(ctx);
    args.push(`-std=${chosen}`);
    for dir in ctx.includes {
        args.push(`-I${dir}`);
    }
    for define in ctx.defines {
        args.push(`-D${define}`);
    }
    for flag in ctx.flags {
        args.push(flag);
    }
    args.push("-MMD");
    args.push("-MF");
    args.push(depfile);
    args.push("-c");
    args.push(src);
    args.push("-o");
    args.push(obj);

    let outputs = [obj, depfile];
    if ctx.profile.coverage {
        let notes = obj;
        notes.replace(".o", ".gcno");
        outputs.push(notes);
    }

    let inputs = [src] + ctx.hdrs + ctx.prior_depfile_headers(obj);

    ctx.action(#{
        name: `compile ${src}`,
        command: compiler_for(ctx, src),
        args: args,
        inputs: inputs,
        execution_deps: [],
        outputs: outputs,
        env: ctx.env,
        toolchain_id: ctx.tool_id(),
    });

    obj;
}

fn archive_one(ctx, objs) {
    let out = ctx.lib_path(ctx.name);

    ctx.action(#{
        name: `archive ${ctx.label}`,
        command: ar_for(ctx),
        args: ["rcs", out] + objs,
        inputs: ctx.hdrs,
        execution_deps: objs,
        outputs: [out],
        env: #{},
        toolchain_id: ctx.tool_id(),
    });
}

fn link_one(ctx, objs, exe_path) {
    let args = objs + ctx.dep_archives;
    if ctx.linker == "" {
        args = args + profile_args(ctx);
    }
    for flag in ctx.link_flags {
        args.push(flag);
    }
    for lib in ctx.system_libs {
        args.push("-l");
        args.push(lib);
    }
    args.push("-o");
    args.push(exe_path);

    let inputs = [];

    ctx.action(#{
        name: `link ${ctx.label}`,
        command: linker_for(ctx),
        args: args,
        inputs: inputs,
        execution_deps: objs + ctx.dep_archives,
        outputs: [exe_path],
        env: #{},
        toolchain_id: ctx.tool_id(),
    });
}

fn build(ctx) {
    if ctx.kind == "library" && ctx.srcs.len() == 0 {
        throw `library ${ctx.label} declares no sources`;
    }

    let objs = [];
    for src in ctx.srcs {
        objs.push(compile_one(ctx, src));
    }

    if ctx.kind == "library" {
        archive_one(ctx, objs);
        return;
    }

    let out_dir = if ctx.kind == "test" { "forge-out/test" } else { "forge-out/bin" };
    let exe = `${out_dir}/${ctx.profile.name}/${ctx.name}`;

    link_one(ctx, objs, exe);

    if ctx.kind == "test" {
        let test_env = ctx.env;
        let test_outputs = [];
        if ctx.profile.coverage {
            test_outputs.push("forge-out/profile/coverage/");
        }
        ctx.action(#{
            name: `run ${ctx.label}`,
            command: exe,
            args: ctx.run_args,
            inputs: ctx.data,
            execution_deps: [exe],
            outputs: test_outputs,
            env: test_env,
            toolchain_id: "",
        });
    }
}
