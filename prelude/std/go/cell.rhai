fn base_env(ctx, cgo_on) {
	let env = #{};
	for key in ctx.env.keys() {
		env[key] = ctx.env[key];
	}
	env["GOTOOLCHAIN"] = "local";
	env["GOENV"] = "off";
	env["GOPROXY"] = "off";
	env["GOOS"] = goos_of(ctx.platform_os);
	env["GOARCH"] = goarch_of(ctx.platform_arch);
	env["CGO_ENABLED"] = if cgo_on { "1" } else { "0" };
	env["GOCACHE"] = "FORGE_EXEC_ROOT/tmp/gocache";
	env
}

fn go_driver(ctx) {
	let driver = ctx.bin("go");
	if driver == "" {
		throw `toolchain provides no go driver`;
	}
	driver
}

fn c_toolchain_present(ctx) {
	for name in ["clang", "gcc", "cc"] {
		if ctx.tool_bin(name) != "" {
			return true;
		}
	}
	false
}

fn c_compiler(ctx) {
	for name in ["clang", "gcc", "cc"] {
		let path = ctx.tool_bin(name);
		if path != "" {
			return path;
		}
	}
	throw `cgo needs a C toolchain providing clang, gcc, or cc`;
}

fn import_of(ctx, name) {
	let cell_config = ctx["cell_config"];
	let table = if cell_config ==() { () } else { cell_config["packages"] };
	if table !=() && table.type_of() == "map" && table[name] !=() {
		return table[name];
	}
	name
}

fn dep_import(ctx, name) {
	import_of(ctx, name)
}

fn dep_imports(ctx) {
	let imports = [];
	for dep in ctx.dep_artifacts {
		let importpath = dep_import(ctx, dep[0]);
		if !imports.contains(importpath) {
			imports.push(importpath);
		}
	}
	imports
}

fn user_imports(ctx, files) {
	let deps = dep_imports(ctx);
	let roots = [];
	for file in files {
		let text = ctx.read_file(file);
		for imp in scan_imports(text) {
			if imp == "C" || imp == "unsafe" || imp == "builtin" {
				continue;
			}
			if deps.contains(imp) {
				continue;
			}
			if imp.contains(".") && imp.split("/")[0].contains(".") {
				throw `external module import ${imp} needs vendored sources`;
			}
			if !roots.contains(imp) {
				roots.push(imp);
			}
		}
	}
	roots
}

fn dep_entries(ctx, std) {
	let entries = [];
	for key in std["cfg"].keys() {
		entries.push([key, std["cfg"][key]]);
	}
	for dep in ctx.dep_artifacts {
		entries.push([dep_import(ctx, dep[0]), dep[1]]);
	}
	entries
}

fn user_sources(ctx, files) {
	let gofiles = [];
	let asm = [];
	let c = [];
	let headers = [];
	for file in files {
		if file.ends_with(".go") {
			gofiles.push(file);
		} else if file.ends_with(".s") {
			asm.push(file);
		} else if file.ends_with(".S") || file.ends_with(".c") {
			c.push(file);
		} else if file.ends_with(".h") {
			headers.push(file);
		} else {
			throw `unsupported Go source: ${file}`;
		}
	}
	#{ gofiles: gofiles, asm: asm, c: c, headers: headers }
}

fn emit_cgo(ctx, driver, opts) {
	let importpath = opts["importpath"];
	let gen = opts["gen"];
	let out = opts["out"];
	let cc = c_compiler(ctx);
	let env = base_env(ctx, true);
	env["CC"] = cc;
	let cgo_args = ["tool", "cgo", "-objdir", `${gen}/`, "-importpath", importpath];
	if opts["no_runtime_cgo"] {
		cgo_args.push("-import_runtime_cgo=false");
	}
	if opts["no_syscall"] {
		cgo_args.push("-import_syscall=false");
	}
	cgo_args += ["-trimpath", "FORGE_EXEC_ROOT=>"] + opts["cgo_files"];
	ctx.action(#{
			name: `go cgo ${importpath}`,
			command: driver,
			args: cgo_args,
			inputs: opts["inputs"],
			execution_deps: [],
			outputs: [`${gen}/`],
			env: env,
			toolchain_id: ctx.tool_id(),
			writes: #{},
		});
	let cobjs = [];
	let main_obj = "";
	let pairs = [];
	for file in opts["cgo_files"] {
		let stem = basename(file);
		pairs.push([`${gen}/${stem.sub_string(0, stem.len() - 3)}.cgo2.c`, dirname(file)]);
	}
	pairs.push([`${gen}/_cgo_export.c`, gen]);
	pairs.push([`${gen}/_cgo_main.c`, gen]);
	for file in opts["c_files"] {
		pairs.push([file, dirname(file)]);
	}
	for pair in pairs {
		let obj = `${opts["out"]}.${basename(pair[0])}.o`;
		let includes = [gen];
		if pair[1] != "" && pair[1] != gen && !includes.contains(pair[1]) {
			includes.push(pair[1]);
		}
		let cc_inputs = [];
		if !opts["is_std"] {
			cc_inputs = [pair[0]];
		}
		emit_cc(ctx, cc, `go cc ${importpath} ${basename(pair[0])}`,
			pair[0], obj, includes, true, cc_inputs, [gen]);
		if pair[0] == `${gen}/_cgo_main.c` {
			main_obj = obj;
		} else {
			cobjs.push(obj);
		}
	}
	let dynobj = `${gen}/_cgo_.o`;
	let link_args = ["-o", dynobj, main_obj] + cobjs;
	if opts["lpthread"] {
		link_args.push("-lpthread");
	}
	let link_inputs = [];
	if !opts["is_std"] {
		link_inputs = [main_obj] + cobjs;
	}
	ctx.action(#{
			name: `go cgo link ${importpath}`,
			command: cc,
			args: link_args,
			inputs: link_inputs,
			execution_deps: [main_obj] + cobjs + [gen],
			outputs: [dynobj],
			env: base_env(ctx, true),
			toolchain_id: ctx.tool_id(),
			writes: #{},
		});
	let import_go = `${gen}/_cgo_import.go`;
	let dyn_args = ["tool", "cgo", "-dynpackage", basename(importpath), "-dynimport", dynobj, "-dynout", import_go];
	if opts["dynlinker"] {
		dyn_args.push("-dynlinker");
	}
	ctx.action(#{
			name: `go cgo dynimport ${importpath}`,
			command: driver,
			args: dyn_args,
			inputs: [],
			execution_deps: [dynobj],
			outputs: [import_go],
			env: base_env(ctx, true),
			toolchain_id: ctx.tool_id(),
			writes: #{},
		});
	#{ gen: gen, objects: cobjs, import_go: import_go }
}

fn package_build(ctx, driver, importpath, parts, std, out, artifact) {
	let cgo_files = [];
	let plain_go = [];
	for file in parts["gofiles"] {
		if file.ends_with("_test.go") {
			continue;
		}
		let text = ctx.read_file(file);
		if has_cgo_import(text) {
			check_cgo_directives(text);
			cgo_files.push(file);
		} else {
			plain_go.push(file);
		}
	}
	if plain_go.len() == 0 && cgo_files.len() == 0 {
		throw `package ${ctx.label} declares no Go sources`;
	}
	if parts["c"].len() > 0 && cgo_files.len() == 0 {
		throw `package ${ctx.label} has C sources but no cgo import`;
	}
	let cgo =();
	if cgo_files.len() > 0 {
		cgo = emit_cgo(ctx, driver, #{
				importpath: importpath, cgo_files: cgo_files, c_files: parts["c"],
				gen: `${out}.gen`, out: out, inputs: cgo_files, is_std: false,
				no_runtime_cgo: false, no_syscall: false, dynlinker: false, lpthread: false,
			});
	}
	let gos = plain_go;
	if cgo !=() {
		gos.push(`${cgo["gen"]}/_cgo_gotypes.go`);
		for file in cgo_files {
			let stem = basename(file);
			gos.push(`${cgo["gen"]}/${stem.sub_string(0, stem.len() - 3)}.cgo1.go`);
		}
		gos.push(cgo["import_go"]);
	}
	let entries = [];
	for key in std["cfg"].keys() {
		entries.push([key, std["cfg"][key]]);
	}
	for dep in ctx.dep_artifacts {
		entries.push([dep_import(ctx, dep[0]), dep[1]]);
	}
	let deps = [];
	for entry in entries {
		deps.push(entry[1]);
	}
	if cgo !=() {
		deps.push(cgo["gen"]);
		deps.push(cgo["import_go"]);
	}
	let needs_pack = parts["asm"].len() > 0 || (cgo !=() && cgo["objects"].len() > 0);
	let asmhdr = if needs_pack { `${out}.goasm/go_asm.h` } else { "" };
	let asm = #{ symabis: "", objects: [] };
	if parts["asm"].len() > 0 {
		asm = emit_asm(ctx, driver, importpath, parts["asm"], out, [pkg_include(ctx)], asmhdr, false);
		deps.push(asm["symabis"]);
	}
	emit_compile(ctx, driver, #{
			importpath: importpath, srcs: gos,
			cfg_text: importcfg_text(entries), cfg_path: `${out}.importcfg`,
			out: out, is_std: false, inputs: plain_go + parts["headers"], exec_deps: deps,
			cgo_on: cgo !=(), complete: !needs_pack,
			symabis: asm["symabis"], asmhdr: asmhdr,
			artifact: if needs_pack { "" } else { artifact },
		});
	if needs_pack {
		let order = [];
		for obj in asm["objects"] {
			order.push(obj);
		}
		if cgo !=() {
			for obj in cgo["objects"] {
				order.push(obj);
			}
		}
		emit_pack(ctx, driver, out, order, asmhdr, artifact);
	}
	#{ archive: out, cgo: cgo !=() }
}

fn testmain_text(aliases) {
	let text = "package main\n\nimport (\n\t\"testing\"\n";
	for entry in aliases {
		text = text + `  ${entry[0]} "${entry[1]}"` + "\n";
	}
	text = text + ")\n\nfunc main() {\n";
	text = text + "\ttests := []testing.InternalTest{\n";
	for entry in aliases {
		for test in entry[2] {
			text = text + `    {"${test}", ${entry[0]}.${test}},` + "\n";
		}
	}
	text = text + "\t}\n\tbenchmarks := []testing.InternalBenchmark{\n";
	for entry in aliases {
		for bench in entry[3] {
			text = text + `    {"${bench}", ${entry[0]}.${bench}},` + "\n";
		}
	}
	text = text + "\t}\n\texamples := []testing.InternalExample{\n";
	for entry in aliases {
		for example in entry[4] {
			text = text + `    {"${example}", ${entry[0]}.${example}, "", false},` + "\n";
		}
	}
	text = text + "\t}\n\tmatcher := func(pat, str string) (bool, error) { return true, nil; };\n";
	text = text + "\ttesting.Main(matcher, tests, benchmarks, examples)\n}\n";
	text
}

fn resolve_lib(ctx, base) {
	for dep in ctx.dep_artifacts {
		let lib = dep_import(ctx, dep[0]);
		if lib == base || basename(lib) == base {
			return lib;
		}
	}
	throw `test ${ctx.label} has no dep providing package ${base}`;
}

fn lib_archive_of(ctx, lib) {
	for dep in ctx.dep_artifacts {
		if dep_import(ctx, dep[0]) == lib {
			return dep[1];
		}
	}
	""
}

fn build_test(ctx, driver, std, cgo_ready) {
	let lib_files = [];
	let internal = #{};
	let internal_order = [];
	let external = #{};
	let external_order = [];
	for file in ctx.srcs {
		if !file.ends_with(".go") {
			throw `unsupported Go source: ${file}`;
		}
		if file.ends_with("_test.go") {
			let clause = package_clause(ctx.read_file(file));
			if clause == "" {
				throw `test file ${file} declares no package`;
			}
			if clause.ends_with("_test") {
				let base = clause.sub_string(0, clause.len() - 5);
				let lib = resolve_lib(ctx, base);
				if !external.contains(lib) {
					external[lib] = [];
					external_order.push(lib);
				}
				let files = external[lib];
				files.push(file);
				external[lib] = files;
			} else {
				let lib = resolve_lib(ctx, clause);
				if !internal.contains(lib) {
					internal[lib] = [];
					internal_order.push(lib);
				}
				let files = internal[lib];
				files.push(file);
				internal[lib] = files;
			}
		} else {
			lib_files.push(file);
		}
	}
	if lib_files.len() > 0 && internal_order.len() == 0 {
		throw `library sources in test srcs need internal test files`;
	}
	if internal_order.len() > 0 && lib_files.len() == 0 {
		throw `internal test ${ctx.label} needs library sources in srcs`;
	}
	if internal_order.len() > 1 {
		throw `test ${ctx.label} covers several libraries internally`;
	}
	let archives = #{};
	if lib_files.len() > 0 {
		let target = internal_order[0];
		let archive = `forge-out/test/${ctx.profile.name}/${ctx.name}.lib.a`;
		let entries = [];
		for entry in dep_entries(ctx, std) {
			if entry[0] != target {
				entries.push(entry);
			}
		}
		emit_compile(ctx, driver, #{
				importpath: target, srcs: lib_files + internal[target],
				cfg_text: importcfg_text(entries), cfg_path: `${archive}.importcfg`,
				out: archive, is_std: false, inputs: lib_files + internal[target], exec_deps: archives_of(entries),
				cgo_on: false, complete: true, symabis: "", asmhdr: "", artifact: "",
			});
		archives[target] = archive;
	}
	for lib in internal_order {
		if archives.contains(lib) {
			continue;
		}
		archives[lib] = lib_archive_of(ctx, lib);
	}
	for lib in external_order {
		if archives.contains(lib) {
			continue;
		}
		archives[lib] = lib_archive_of(ctx, lib);
	}
	let mains = [];
	for lib in external_order {
		let archive = `forge-out/test/${ctx.profile.name}/${ctx.name}.${basename(lib)}.a`;
		let entries = [];
		for entry in dep_entries(ctx, std) {
			entries.push(entry);
		}
		entries.push([lib, archives[lib]]);
		emit_compile(ctx, driver, #{
				importpath: `${lib}_test`, srcs: external[lib],
				cfg_text: importcfg_text(entries), cfg_path: `${archive}.importcfg`,
				out: archive, is_std: false, inputs: external[lib], exec_deps: archives_of(entries),
				cgo_on: false, complete: true, symabis: "", asmhdr: "", artifact: "",
			});
		mains.push([`${lib}_test`, archive, external[lib]]);
	}
	for lib in internal_order {
		if external.contains(lib) {
			continue;
		}
		mains.push([lib, archives[lib], internal[lib]]);
	}
	if mains.len() == 0 {
		throw `test ${ctx.label} declares no test sources`;
	}
	let aliases = [];
	let counter = 0;
	for entry in mains {
		let tests = [];
		let benchmarks = [];
		let examples = [];
		for file in entry[2] {
			for found in test_funcs(ctx.read_file(file)) {
				if found["kind"] == "Test" {
					tests.push(found["name"]);
				} else if found["kind"] == "Benchmark" {
					benchmarks.push(found["name"]);
				} else {
					examples.push(found["name"]);
				}
			}
		}
		aliases.push([`testimport${counter}`, entry[0], tests, benchmarks, examples]);
		counter += 1;
	}
	let main_src = `forge-out/test/${ctx.profile.name}/${ctx.name}.main.go`;
	let main_archive = `forge-out/test/${ctx.profile.name}/${ctx.name}.main.a`;
	let entries = [];
	for entry in dep_entries(ctx, std) {
		entries.push(entry);
	}
	for entry in mains {
		entries.push([entry[0], entry[1]]);
	}
	let writes = #{};
	writes[main_src] = testmain_text(aliases);
	writes[`${main_archive}.importcfg`] = importcfg_text(entries);
	ctx.action(#{
			name: `go compile main`,
			command: driver,
			args: ["tool", "compile", "-p", "main", "-importcfg", `${main_archive}.importcfg`,
			"-trimpath", "FORGE_EXEC_ROOT=>", "-buildid=", "-pack", "-nolocalimports",
			"-complete", "-o", main_archive, main_src],
			inputs: [],
			execution_deps: archives_of(entries),
			outputs: [main_archive],
			env: base_env(ctx, false),
			toolchain_id: ctx.tool_id(),
			writes: writes,
		});
	let link_archives = [main_archive];
	for dep in ctx.dep_artifacts {
		link_archives.push(dep[1]);
	}
	for entry in mains {
		link_archives.push(entry[1]);
	}
	for archive_path in std["archives"] {
		link_archives.push(archive_path);
	}
	let exe = `forge-out/test/${ctx.profile.name}/${ctx.name}`;
	link_go(
		ctx,
		driver,
		main_archive,
		link_archives,
		[],
		importcfg_text(entries),
		`${exe}.importcfg`,
		exe,
		cgo_ready
	);
	ctx.action(#{
			name: `run ${ctx.label}`,
			command: exe,
			args: ctx.run_args,
			inputs: ctx.data,
			execution_deps: [exe],
			outputs: [],
			is_test: true,
			env: ctx.env,
			toolchain_id: "",
		});
}

fn archives_of(entries) {
	let paths = [];
	for entry in entries {
		if !paths.contains(entry[1]) {
			paths.push(entry[1]);
		}
	}
	paths
}

fn link_go(ctx, driver, main_archive, archives, objects, cfg_text, cfg_path, exe, cgo_on) {
	let args = ["tool", "link", "-buildid=", "-buildmode=exe", "-importcfg", cfg_path, "-o", exe];
	if cgo_on {
		args += ["-linkmode=external", `-extld=${c_compiler(ctx)}`];
	}
	args.push(main_archive);
	for obj in objects {
		args.push(obj);
	}
	let all = [main_archive];
	for archive in archives {
		if !all.contains(archive) {
			all.push(archive);
		}
	}
	ctx.action(#{
			name: `go link ${exe}`,
			command: driver,
			args: args,
			inputs: [],
			execution_deps: all,
			outputs: [exe],
			env: base_env(ctx, cgo_on),
			toolchain_id: ctx.tool_id(),
			writes: writes_for(cfg_path, cfg_text),
		});
}

fn writes_for(path, content) {
	let writes = #{};
	writes[path] = content;
	writes
}

fn build_library(ctx, driver, importpath, std) {
	let parts = user_sources(ctx, ctx.srcs + ctx.hdrs);
	if parts["gofiles"].len() == 0 {
		throw `library ${ctx.label} declares no sources`;
	}
	let out = ctx.lib_path(`lib${ctx.name}.a`);
	package_build(ctx, driver, importpath, parts, std, out, out);
}

fn build_binary(ctx, driver, importpath, std, cgo_ready) {
	let parts = user_sources(ctx, ctx.srcs + ctx.hdrs);
	let out_dir = "forge-out/bin";
	let exe = `${out_dir}/${ctx.profile.name}/${ctx.name}`;
	let archive = `${out_dir}/${ctx.profile.name}/${ctx.name}.a`;
	package_build(ctx, driver, "main", parts, std, archive, "");
	let entries = [];
	for key in std["cfg"].keys() {
		entries.push([key, std["cfg"][key]]);
	}
	for dep in ctx.dep_artifacts {
		entries.push([dep_import(ctx, dep[0]), dep[1]]);
	}
	let link_archives = [];
	for dep in ctx.dep_artifacts {
		link_archives.push(dep[1]);
	}
	for archive_path in std["archives"] {
		link_archives.push(archive_path);
	}
	link_go(
		ctx,
		driver,
		archive,
		link_archives,
		[],
		importcfg_text(entries),
		`${exe}.importcfg`,
		exe,
		cgo_ready
	);
}

fn plan(ctx) { #{} }

fn build(ctx, plan) {
	if ctx.compiler != "" && ctx.compiler != "gccgo" {
		throw `unsupported Go compiler: ${ctx.compiler}`;
	}
	let driver = go_driver(ctx);
	let parts = user_sources(ctx, ctx.srcs + ctx.hdrs);
	if ctx.compiler == "gccgo" {
		build_gccgo(ctx, parts);
		return;
	}
	let scan = parts["gofiles"] + parts["asm"];
	if ctx.kind == "test" {
		scan += ctx.srcs;
	}
	let roots = user_imports(ctx, scan);
	let cgo_probe = false;
	for file in parts["gofiles"] {
		if file.ends_with("_test.go") {
			continue;
		}
		if has_cgo_import(ctx.read_file(file)) {
			cgo_probe = true;
		}
	}
	let cgo_ready = cgo_probe || c_toolchain_present(ctx);
	if cgo_probe && !roots.contains("syscall") {
		roots.push("syscall");
	}
	let std = emit_std_for(ctx, plan, roots, cgo_probe, cgo_ready);
	let importpath = import_of(ctx, ctx.name);
	if ctx.kind == "library" {
		build_library(ctx, driver, importpath, std);
		return;
	}
	if ctx.kind == "test" {
		build_test(ctx, driver, std, cgo_ready);
		return;
	}
	build_binary(ctx, driver, importpath, std, cgo_ready);
}

fn emit_std_for(ctx, plan, roots, cgo_on, cgo_ready) {
	let layout = go_layout(ctx);
	let queue = roots;
	if cgo_ready {
		queue.push("runtime/cgo");
	}
	let closure = std_closure(ctx, layout, queue, cgo_on);
	emit_std(ctx, plan, layout, closure)
}

fn build_gccgo(ctx, parts) {
	let driver = ctx.tool_bin("gccgo");
	if driver == "" {
		throw `compiler gccgo needs a gcc toolchain built with Go support`;
	}
	let importpath = import_of(ctx, ctx.name);
	let dirs = [];
	for dep in ctx.dep_artifacts {
		let dir = dirname(dep[1]);
		if dir != "" && !dirs.contains(dir) {
			dirs.push(dir);
		}
	}
	if ctx.kind == "library" {
		let out = ctx.lib_path(`lib${ctx.name}.a`);
		let args = ["-c", "-O2", `-fgo-pkgpath=${importpath}`, "-o", out];
		for dir in dirs {
			args += ["-I", dir];
		}
		for file in parts["gofiles"] {
			args.push(file);
		}
		ctx.action(#{
				name: `gccgo ${ctx.label}`,
				command: driver,
				args: args,
				inputs: parts["gofiles"],
				execution_deps: [],
				outputs: [out],
				artifact: out,
				env: base_env(ctx, false),
				toolchain_id: ctx.tool_id(),
				writes: #{},
			});
		return;
	}
	throw `gccgo backend supports libraries only`;
}
