fn goos_of(os) {
	if os == "linux" {
		"linux"
	} else if os == "darwin" || os == "macos" {
		"darwin"
	} else if os == "windows" {
		"windows"
	} else {
		throw `unsupported Go OS: ${os}`;
	}
}

fn goarch_of(arch) {
	if arch == "x86_64" || arch == "amd64" {
		"amd64"
	} else if arch == "aarch64" || arch == "arm64" {
		"arm64"
	} else {
		throw `unsupported Go arch: ${arch}`;
	}
}

fn go_layout(ctx) {
	let root = ctx.toolchain_dir("go");
	if root == "" {
		throw `toolchain provides no go root`;
	}
	if ctx.toolchain_glob("go", "go/src/runtime/*.go").len() > 0 {
		return #{ root: `${root}/go`, src: `${root}/go/src`, rel: "go/src", top: root };
	}
	if ctx.toolchain_glob("go", "src/runtime/*.go").len() > 0 {
		return #{ root: root, src: `${root}/src`, rel: "src", top: root };
	}
	throw `go toolchain has no src tree`;
}

fn pkg_include(ctx) {
	`${go_layout(ctx)["root"]}/pkg/include`
}

fn std_out(ctx, importpath) {
	`forge-out/lib/deps/${ctx.profile.name}/std-${goos_of(ctx.platform_os)}-${goarch_of(ctx.platform_arch)}/${importpath}.a`
}

fn importcfg_text(entries) {
	let text = "";
	for entry in entries {
		text = text + `packagefile ${entry[0]}=${entry[1]}` + "\n";
	}
	text
}

fn linknamed_std(importpath) {
	["bytes", "internal/poll", "net", "os", "runtime/metrics", "runtime/pprof",
		"runtime/trace", "sync", "syscall", "time"].contains(importpath)
}

fn emit_compile(ctx, driver, opts) {
	let importpath = opts["importpath"];
	let out = opts["out"];
	let cfg_path = opts["cfg_path"];
	let args = ["tool", "compile", "-p", importpath, "-importcfg", cfg_path,
		"-trimpath", "FORGE_EXEC_ROOT=>", "-buildid=", "-pack", "-nolocalimports"];
	if opts["is_std"] {
		args.push("-std");
	}
	if !opts["cgo_on"] && opts["complete"] {
		args.push("-complete");
	}
	if opts["symabis"] != "" {
		args += ["-symabis", opts["symabis"]];
	}
	if opts["asmhdr"] != "" {
		args += ["-asmhdr", opts["asmhdr"]];
	}
	args += ["-o", out];
	for src in opts["srcs"] {
		args.push(src);
	}
	let writes = #{};
	writes[cfg_path] = opts["cfg_text"];
	let outputs = [out];
	if opts["asmhdr"] != "" {
		writes[opts["asmhdr"]] = "";
		outputs.push(opts["asmhdr"]);
	}
	let action = #{
		name: `go compile ${importpath}`,
		command: driver,
		args: args,
		inputs: opts["inputs"],
		execution_deps: opts["exec_deps"],
		outputs: outputs,
		env: base_env(ctx, opts["cgo_on"]),
		toolchain_id: ctx.tool_id(),
		writes: writes,
	};
	if opts["artifact"] != "" {
		action["artifact"] = opts["artifact"];
	}
	ctx.action(action);
}

fn emit_asm(ctx, driver, importpath, files, out, include_dirs, asmhdr, is_std) {
	let symabis = `${out}.symabis`;
	let sym_args = ["tool", "asm", "-p", importpath, "-trimpath", "FORGE_EXEC_ROOT=>"];
	for dir in include_dirs {
		sym_args += ["-I", dir];
	}
	sym_args += ["-D", `GOOS_${goos_of(ctx.platform_os)}`, "-D", `GOARCH_${goarch_of(ctx.platform_arch)}`];
	if is_std {
		sym_args.push("-std");
	}
	if goarch_of(ctx.platform_arch) == "amd64" {
		sym_args += ["-D", "GOAMD64_v1"];
	}
	let sym_writes = #{};
	if asmhdr != "" {
		sym_args += ["-I", dirname(asmhdr)];
		sym_writes[asmhdr] = "";
	}
	sym_args += ["-gensymabis", "-o", symabis];
	for file in files {
		sym_args.push(file);
	}
	let sym_inputs = [];
	if !is_std {
		sym_inputs = files;
	}
	ctx.action(#{
		name: `go asm ${importpath}`,
		command: driver,
		args: sym_args,
		inputs: sym_inputs,
		execution_deps: [],
		outputs: [symabis],
		env: base_env(ctx, false),
		toolchain_id: ctx.tool_id(),
		writes: sym_writes,
	});
	let objects = [];
	for file in files {
		let obj = `${out}.${basename(file)}.o`;
		let obj_args = ["tool", "asm", "-p", importpath, "-trimpath", "FORGE_EXEC_ROOT=>"];
		for dir in include_dirs {
			obj_args += ["-I", dir];
		}
		let obj_deps = [symabis];
		if asmhdr != "" {
			obj_args += ["-I", dirname(asmhdr)];
			obj_deps.push(asmhdr);
		}
		if is_std {
			obj_args.push("-std");
		}
		let obj_inputs = [];
		if !is_std {
			obj_inputs = [file];
		}
		obj_args += ["-D", `GOOS_${goos_of(ctx.platform_os)}`, "-D", `GOARCH_${goarch_of(ctx.platform_arch)}`, "-o", obj, file];
		ctx.action(#{
			name: `go asm ${importpath} ${basename(file)}`,
			command: driver,
			args: obj_args,
			inputs: obj_inputs,
			execution_deps: obj_deps,
			outputs: [obj],
			env: base_env(ctx, false),
			toolchain_id: ctx.tool_id(),
			writes: #{},
		});
		objects.push(obj);
	}
	#{ symabis: symabis, objects: objects }
}

fn emit_cc(ctx, cc, name, src, obj, include_dirs, cgo_on, inputs, exec_deps) {
	let args = ["-c", "-O2", "-g", "-fPIC", "-pthread", "-fno-stack-protector",
		"-gno-record-gcc-switches", `-ffile-prefix-map=FORGE_EXEC_ROOT=>`];
	for dir in include_dirs {
		args += ["-I", dir];
	}
	args += ["-o", obj, src];
	ctx.action(#{
		name: name,
		command: cc,
		args: args,
		inputs: inputs,
		execution_deps: exec_deps,
		outputs: [obj],
		env: base_env(ctx, cgo_on),
		toolchain_id: ctx.tool_id(),
		writes: #{},
	});
	obj
}

fn emit_pack(ctx, driver, archive, objects, order_dep, artifact) {
	let action = #{
		name: `go pack ${archive}`,
		command: driver,
		args: ["tool", "pack", "r", archive] + objects,
		inputs: [archive] + objects,
		execution_deps: [order_dep] + objects,
		outputs: [archive],
		env: base_env(ctx, false),
		toolchain_id: ctx.tool_id(),
		writes: #{},
	};
	if artifact != "" {
		action["artifact"] = artifact;
	}
	ctx.action(action);
}

fn std_members(ctx, layout, importpath, cgo_on) {
	let dir = `${layout["rel"]}/${importpath}`;
	let top = layout["top"];
	let files = [];
	let cgofiles = [];
	let asm = [];
	let c = [];
	let imports = [];
	for hit in ctx.toolchain_glob("go", `${dir}/*.go`) {
		if name_excluded(ctx, hit) {
			continue;
		}
		let text = ctx.toolchain_read("go", hit);
		if !src_kept(ctx, cgo_on, hit, text) {
			continue;
		}
		if has_cgo_import(text) {
			cgofiles.push(`${top}/${hit}`);
		} else {
			files.push(`${top}/${hit}`);
		}
		for imp in scan_imports(text) {
			if imp != "C" && !imports.contains(imp) {
				imports.push(imp);
			}
		}
	}
	for hit in ctx.toolchain_glob("go", `${dir}/*.s`) {
		if name_excluded(ctx, hit) {
			continue;
		}
		let text = ctx.toolchain_read("go", hit);
		if !src_kept(ctx, cgo_on, hit, text) {
			continue;
		}
		asm.push(`${top}/${hit}`);
	}
	for pattern in [`${dir}/*.c`, `${dir}/*.S`] {
		for hit in ctx.toolchain_glob("go", pattern) {
			if name_excluded(ctx, hit) {
				continue;
			}
			let text = ctx.toolchain_read("go", hit);
			if !src_kept(ctx, cgo_on, hit, text) {
				continue;
			}
			c.push(`${top}/${hit}`);
		}
	}
	#{ files: files, cgofiles: cgofiles, asm: asm, c: c, imports: imports }
}

fn std_closure(ctx, layout, roots, cgo_on) {
	let closure = #{};
	let queue = [];
	for root in roots {
		if root == "C" || root == "unsafe" || root == "builtin" {
			continue;
		}
		if root.starts_with("cmd/") {
			continue;
		}
		if !closure.contains(root) {
			closure[root] = ();
			queue.push(root);
		}
	}
	if cgo_on && !closure.contains("runtime/cgo") {
		closure["runtime/cgo"] = ();
		queue.push("runtime/cgo");
	}
	while queue.len() > 0 {
		let next = queue.pop();
		let members = std_members(ctx, layout, next, cgo_on);
		if members["files"].len() == 0 && members["cgofiles"].len() == 0 {
			throw `cannot resolve std import ${next}`;
		}
		closure[next] = members;
		for imp in members["imports"] {
			if imp == "C" || imp == "unsafe" || imp == "builtin" {
				continue;
			}
			if imp.starts_with("cmd/") {
				continue;
			}
			if !closure.contains(imp) {
				closure[imp] = ();
				queue.push(imp);
			}
		}
	}
	closure
}

fn emit_std(ctx, plan, layout, closure) {
	let driver = go_driver(ctx);
	let cc = ();
	let tag = `go std ${goos_of(ctx.platform_os)} ${goarch_of(ctx.platform_arch)} ${ctx.profile.name}`;
	let cfg = #{};
	let archives = [];
	for importpath in closure.keys() {
		let members = closure[importpath];
		if members.type_of() != "map" {
			continue;
		}
		let out = std_out(ctx, importpath);
		cfg[importpath] = out;
		archives.push(out);
		if !plan.once(`${tag} ${importpath}`) {
			continue;
		}
		let entries = [];
		for imp in members["imports"] {
			if imp == "C" || imp == "unsafe" || imp == "builtin" {
				continue;
			}
			entries.push([imp, std_out(ctx, imp)]);
		}
		let deps = [];
		for entry in entries {
			deps.push(entry[1]);
		}
		let needs_pack = members["asm"].len() > 0 || members["c"].len() > 0;
		let asmhdr = if needs_pack { `${out}.goasm/go_asm.h` } else { "" };
		let asm = #{ symabis: "", objects: [] };
		if members["asm"].len() > 0 {
			asm = emit_asm(ctx, driver, importpath, members["asm"], out, [pkg_include(ctx)], asmhdr, true);
			deps.push(asm["symabis"]);
		}
		let cobjs = [];
		let cgo = ();
		if members["cgofiles"].len() > 0 {
			let special = importpath == "runtime/cgo";
			cgo = emit_cgo(ctx, driver, #{
				importpath: importpath, cgo_files: members["cgofiles"], c_files: members["c"],
				gen: `${out}.gen`, out: out, inputs: [], is_std: true,
				no_runtime_cgo: special, no_syscall: special, dynlinker: special, lpthread: special,
			});
		} else if members["c"].len() > 0 {
			if cc == () {
				cc = c_compiler(ctx);
			}
			let pkgdir = `${layout["root"]}/${layout["rel"]}/${importpath}`;
			for src in members["c"] {
				cobjs.push(emit_cc(ctx, cc, `go cc ${importpath} ${basename(src)}`,
					src, `${out}.${basename(src)}.o`, [pkgdir, pkg_include(ctx)], false, [], []));
			}
		}
		let gos = members["files"];
		if cgo != () {
			gos.push(`${cgo["gen"]}/_cgo_gotypes.go`);
			for file in members["cgofiles"] {
				let stem = basename(file);
				gos.push(`${cgo["gen"]}/${stem.sub_string(0, stem.len() - 3)}.cgo1.go`);
			}
			gos.push(cgo["import_go"]);
			deps.push(cgo["gen"]);
			deps.push(cgo["import_go"]);
		}
		emit_compile(ctx, driver, #{
			importpath: importpath, srcs: gos,
			cfg_text: importcfg_text(entries), cfg_path: `${out}.importcfg`,
			out: out, is_std: true, inputs: [], exec_deps: deps,
			cgo_on: cgo != (), complete: !needs_pack && !linknamed_std(importpath),
			symabis: asm["symabis"], asmhdr: asmhdr, artifact: "",
		});
		if needs_pack {
			let order = [];
			for obj in asm["objects"] {
				order.push(obj);
			}
			for obj in cobjs {
				order.push(obj);
			}
			if cgo != () {
				for obj in cgo["objects"] {
					order.push(obj);
				}
			}
			emit_pack(ctx, driver, out, order, asmhdr, "");
		}
	}
	#{ cfg: cfg, archives: archives }
}
