fn is_cpp(src) {
    src.ends_with(".cpp") || src.ends_with(".cc") || src.ends_with(".cxx") || src.ends_with(".C")
        || is_module(src);
}

fn is_module(src) {
    src.ends_with(".cppm") || src.ends_with(".ixx") || src.ends_with(".mpp") || src.ends_with(".cxxm");
}

fn module_name_of(ctx, src) {
    if !is_module(src) { return ""; }
    let text = ctx.read_file(src);
    if !text.contains("export module ") { return ""; }
    text.split("export module ")[1].split(";")[0];
}

fn coverage_dir(ctx) {
    `forge-out/profile/${ctx.profile.name}`
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

fn lto_archiver(ctx) {
    let compiler = if ctx.srcs.len() > 0 { compiler_for(ctx, ctx.srcs[0]) } else { "" };
    let candidates = if compiler.contains("clang") { ["llvm-ar", "gcc-ar", "ar"] } else { ["gcc-ar", "ar"] };
    for name in candidates {
        let path = ctx.bin(name);
        if path != "" {
            return path;
        }
    }
    throw `toolchain provides no LTO-capable archiver`;
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

fn opt_flag(level) {
    if level == "s" { "-Os" }
    else if level == "z" { "-Oz" }
    else { `-O${level}` }
}

fn debug_flag(info) {
    if info == "full" { "-g" }
    else if info == "limited" { "-g1" }
    else if info == "line-tables-only" { "-gline-tables-only" }
    else { "" }
}

fn profile_args(c, llvm) {
    let args = [opt_flag(c.profile.opt_level), "-ffile-prefix-map=FORGE_EXEC_ROOT=."];
    let info = debug_flag(c.profile["debug"]);
    if info != "" {
        args.push(info);
    }
    for san in c.profile.sanitizers {
        args.push(`-fsanitize=${san}`);
    }
    if c.profile.coverage {
        if llvm {
            args.push("-fprofile-instr-generate");
            args.push("-fcoverage-mapping");
        } else {
            args.push("--coverage");
        }
    }
    if c.profile.lto == "thin" { args.push("-flto=thin"); }
    else if c.profile.lto == "fat" { args.push("-flto"); }
    for define in c.profile.defines {
        args.push(`-D${define}`);
    }
    args;
}

fn compile_one(ctx, src, module_flags) {
    let default_std = if is_cpp(src) { "c++20" } else { "c17" };
    let chosen = if ctx.standard == "" { default_std } else { ctx.standard };

    let category = if ctx.profile.coverage { "profile" } else { "obj" };
    let stem = ctx.artifact_path(src, category);
    let obj = `${stem}.o`;
    let depfile = `${obj}.d`;
    let compiler = compiler_for(ctx, src);
    let llvm = compiler.contains("clang");

    let args = profile_args(ctx, llvm);
    let module_deps = [];
    for flag in module_flags {
        args.push(flag);
        let parts = flag.split("=");
        module_deps.push(parts[parts.len() - 1]);
    }
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
    if ctx.profile.coverage && !llvm {
        outputs.push(`${stem}.gcno`);
    }

    let inputs = [src] + ctx.hdrs + ctx.depfile_inputs(depfile);

    ctx.action(#{
        name: `compile ${src}`,
        command: compiler,
        args: args,
        inputs: inputs,
        execution_deps: module_deps,
        outputs: outputs,
        env: ctx.env,
        toolchain_id: ctx.tool_id(),
    });

    obj;
}

fn archive_one(ctx, objs) {
    let out = ctx.lib_path(`lib${ctx.name}.a`);
    let lto = ctx.profile.lto == "thin" || ctx.profile.lto == "fat";
    let archiver = if lto { lto_archiver(ctx) } else { ar_for(ctx) };

    ctx.action(#{
        name: `archive ${ctx.label}`,
        command: archiver,
        args: ["rcsD", out] + objs,
        inputs: ctx.hdrs,
        execution_deps: objs,
        outputs: [out],
        artifact: out,
        env: #{},
        toolchain_id: ctx.tool_id(),
    });
}

fn link_one(ctx, objs, exe_path) {
    let linker = linker_for(ctx);
    let args = objs + ctx.dep_archives;
    if ctx.linker == "" {
        args = args + profile_args(ctx, linker.contains("clang"));
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
        command: linker,
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

    let module_flags = [];
    for src in ctx.srcs {
        let name = module_name_of(ctx, src);
        if name == "" { continue; }
        let compiler = compiler_for(ctx, src);
        if !compiler.contains("clang") {
            throw `module interface ${src} requires clang, found ${compiler}`;
        }
        let pcm = `forge-out/mod/${ctx.profile.name}/${name}.pcm`;
        let std = if ctx.standard == "" { "c++20" } else { ctx.standard };
        let args = profile_args(ctx, true);
        args.push(`-std=${std}`);
        args.push("-x");
        args.push("c++-module");
        args.push("--precompile");
        args.push(src);
        args.push("-o");
        args.push(pcm);
        for dir in ctx.includes {
            args.push(`-I${dir}`);
        }
        ctx.action(#{
            name: `precompile module ${name}`,
            command: compiler,
            args: args,
            inputs: [src] + ctx.hdrs,
            execution_deps: [],
            outputs: [pcm],
            env: ctx.env,
            toolchain_id: ctx.tool_id(),
        });
        module_flags.push(`-fmodule-file=${name}=${pcm}`);
    }

    let objs = [];
    for src in ctx.srcs {
        objs.push(compile_one(ctx, src, module_flags));
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
            test_outputs.push(`${coverage_dir(ctx)}/`);
            if linker_for(ctx).contains("clang") {
                test_env["LLVM_PROFILE_FILE"] = `${coverage_dir(ctx)}/%m.profraw`;
            }
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
