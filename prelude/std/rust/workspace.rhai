fn rust_mode(config) {
    if config == () { return "native"; }
    if config.type_of() != "map" { throw "cell.rust must be a table"; }
    for key in config.keys() {
        if !["mode", "dependencies"].contains(key) { throw `unknown cell.rust key: ${key}`; }
    }
    let mode = if config["mode"] == () { "native" } else { config["mode"] };
    if mode != "native" && mode != "cargo" { throw `unknown cell.rust mode: ${mode}`; }
    mode
}

fn rust_name(name) {
    if name.type_of() != "string" || name == "" { throw "Rust crate name must be a non-empty string"; }
    name.replace("-", "_");
    for ch in name.chars() {
        if !"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".contains(ch) {
            throw `invalid Rust crate name: ${name}`;
        }
    }
    name
}

fn rust_path(root, path) {
    if path.type_of() != "string" || path.starts_with("/") || path.contains("\\") || path.contains(":") {
        throw `expected workspace-relative Rust path: ${path}`;
    }
    let parts = [];
    for part in (root + "/" + path).split("/") {
        if part == "" || part == "." { continue; }
        if part == ".." {
            if parts.len() == 0 { throw `Rust path leaves workspace: ${root}/${path}`; }
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    let joined = "";
    for part in parts {
        if joined != "" { joined += "/"; }
        joined += part;
    }
    joined
}

fn cargo_glob(pattern) {
    let files = [];
    try { files = glob(pattern); } catch (error) {
        if !error.to_string().contains("matched nothing") { throw error; }
    }
    files
}

fn cargo_members(root) {
    let paths = [];
    if root["package"] != () { paths.push("Cargo.toml"); }
    let workspace = root["workspace"];
    if workspace == () { return paths; }
    let excluded = [];
    if workspace["exclude"] != () {
        for pattern in workspace["exclude"] {
            excluded += cargo_glob(rust_path(pattern, "Cargo.toml"));
        }
    }
    if workspace["members"] != () {
        for pattern in workspace["members"] {
            let matches = cargo_glob(rust_path(pattern, "Cargo.toml"));
            if matches.len() == 0 { throw `workspace member matched no Cargo.toml: ${pattern}`; }
            for path in matches {
                if !excluded.contains(path) && !paths.contains(path) { paths.push(path); }
            }
        }
    }
    paths
}

fn cargo_library(manifest, root) {
    let lib = if manifest["lib"] == () { #{} } else { manifest["lib"] };
    let path = rust_path(root, if lib["path"] == () { "src/lib.rs" } else { lib["path"] });
    if manifest["lib"] == () && !path_exists(path) { return (); }
    if !path_exists(path) { throw `Cargo library source does not exist: ${path}`; }
    if lib["proc-macro"] == true || (lib["crate-type"] != () && lib["crate-type"] != ["lib"] && lib["crate-type"] != ["rlib"]) {
        throw "Cargo importer currently supports local rlib libraries only";
    }
    #{ name: rust_name(if lib["name"] == () { manifest["package"]["name"] } else { lib["name"] }), path: path }
}

fn cargo_emit(name, kind, path, root, manifest, deps, aliases, used) {
    if used.contains(name) { throw `Cargo target name collision: ${name}; use manual FORGE targets`;
    }
    if !path_exists(path) { throw `Cargo target source does not exist: ${path}`; }
    let edition = if manifest["package"]["edition"] == () { "2015" } else { manifest["package"]["edition"] };
    let srcs = cargo_glob(rust_path(root, "src/**/*.rs"));
    if !srcs.contains(path) { srcs.push(path); }
    let fields = #{
        srcs: srcs,
        hdrs: [rust_path(root, "Cargo.toml"), "Cargo.toml"],
        deps: deps,
        standard: edition,
        metadata: #{ rust: #{ root: root, crate_root: path, name: rust_name(if kind == "library" { name } else { manifest["forge-bin-name"] }), manifest: manifest, manifest_path: rust_path(root, "Cargo.toml"), externs: aliases } },
    };
    if kind == "library" { library(name, fields); } else { binary(name, fields); }
    used.push(name);
    used
}

fn cargo_locked_dependencies(manifest, lock) {
    let owner = ();
    let pkg = manifest["package"];
    let pins = if lock["package"] == () { [] } else { lock["package"] };
    for pin in pins {
        if pin["source"] == () && pin["name"] == pkg["name"] && pin["version"] == pkg["version"] { owner = pin; }
    }
    let selected = #{};
    for section_name in ["dependencies", "build-dependencies"] {
        let section = manifest[section_name];
        if section == () { continue; }
        for alias in section.keys() {
            let spec = section[alias];
            if spec.type_of() == "string" { spec = #{ version: spec }; }
            if spec["path"] != () { continue; }
            let name = if spec["package"] == () { alias } else { spec["package"] };
            if owner == () || owner["dependencies"] == () { throw `Cargo.lock is missing dependencies for ${pkg["name"]}; regenerate it with Cargo`; }
            let matches = [];
            for edge in owner["dependencies"] {
                let fields = edge.split(" ");
                if fields[0] != name { continue; }
                for pin in pins {
                    if pin["name"] != name || pin["source"] == () { continue; }
                    if fields.len() > 1 && fields[1] != pin["version"] { continue; }
                    if fields.len() > 2 && fields[2] != `(${pin["source"]})` { continue; }
                    let source = pin["source"];
                    if spec["git"] != () {
                        if !source.starts_with("git+") || source.sub_string(4).split("#")[0].split("?")[0] != spec["git"] { continue; }
                    } else if source != "registry+https://github.com/rust-lang/crates.io-index" && source != "sparse+https://index.crates.io/" { continue; }
                    if !matches.contains(pin["version"]) { matches.push(pin["version"]); }
                }
            }
            if matches.len() != 1 { throw `Cargo.lock has no unambiguous source-matching pin for ${pkg["name"]} dependency ${alias}`; }
            if selected[name] != () && selected[name] != matches[0] { throw `multiple direct versions of ${name} are not yet supported`; }
            selected[name] = matches[0];
        }
    }
    manifest["forge-pins"] = selected;
    manifest
}

fn cargo_import_targets(root, lock) {
    let workspace = if root["workspace"] == () { #{} } else { root["workspace"] };
    let paths = cargo_members(root);
    let packages = #{};
    let index = 0;
    while index < paths.len() {
        let path = paths[index];
        index += 1;
        let directory = path.split("/");
        directory.pop();
        let joined = "";
        for part in directory {
            if joined != "" { joined += "/"; }
            joined += part;
        }
        let directory = joined;
        let manifest = cargo_inherit(toml_decode(read_file(path)), workspace);
        if manifest["target"] != () { throw "Cargo importer does not yet support local target-specific dependencies; use manual FORGE targets"; }
        for section_name in ["dependencies", "build-dependencies"] {
            let section = manifest[section_name];
            if section == () { continue; }
            for name in section.keys() {
                let spec = section[name];
                if spec.type_of() != "map" { continue; }
                if spec["optional"] == true { throw "Cargo importer does not yet resolve local optional dependencies"; }
                if spec["registry"] != () || spec["registry-index"] != () { throw "Cargo importer supports only the default crates.io registry"; }
                if spec["path"] == () { continue; }
                if section_name == "build-dependencies" { throw "Cargo importer does not yet support local path build-dependencies"; }
                let dep_path = rust_path(if spec["forge-workspace-path"] == true { "" } else { directory }, spec["path"]);
                let dep_manifest = rust_path(dep_path, "Cargo.toml");
                if !paths.contains(dep_manifest) { paths.push(dep_manifest); }
            }
        }
        manifest = cargo_locked_dependencies(manifest, lock);
        packages[directory] = #{ manifest: manifest, lib: cargo_library(manifest, directory) };
    }
    let used = [];
    for directory in packages.keys() {
        let manifest = packages[directory]["manifest"];
        let lib = packages[directory]["lib"];
        let deps = [];
        let aliases = #{};
        let section = manifest["dependencies"];
        if section != () {
            for alias in section.keys() {
                let spec = section[alias];
                if spec.type_of() != "map" || spec["path"] == () { continue; }
                let dep_root = rust_path(if spec["forge-workspace-path"] == true { "" } else { directory }, spec["path"]);
                let dep_lib = packages[dep_root]["lib"];
                if dep_lib == () { throw `path dependency ${alias} has no library`; }
                let expected = if spec["package"] == () { alias } else { spec["package"] };
                if packages[dep_root]["manifest"]["package"]["name"] != expected { throw `path dependency package name mismatch: ${alias}`; }
                deps.push(dep_lib["name"]);
                aliases[dep_lib["name"]] = rust_name(alias);
            }
        }
        if lib != () {
            used = cargo_emit(lib["name"], "library", lib["path"], directory, manifest, deps, aliases, used);
            deps.push(lib["name"]);
        }
        let bins = if manifest["bin"] == () { [] } else { manifest["bin"] };
        let bin_names = [];
        for bin in bins { bin_names.push(bin["name"]); }
        if manifest["package"]["autobins"] != false {
            let package_name = manifest["package"]["name"];
            if path_exists(rust_path(directory, "src/main.rs")) && !bin_names.contains(package_name) {
                bins.push(#{ name: package_name, path: "src/main.rs" });
                bin_names.push(package_name);
            }
            for path in cargo_glob(rust_path(directory, "src/bin/*.rs")) + cargo_glob(rust_path(directory, "src/bin/*/main.rs")) {
                let parts = path.split("/");
                let filename = parts.pop();
                let name = if filename == "main.rs" { parts.pop() } else { filename.sub_string(0, filename.len() - 3) };
                if bin_names.contains(name) { continue; }
                let relative = if directory == "" { path } else { path.sub_string(directory.len() + 1) };
                bins.push(#{ name: name, path: relative });
                bin_names.push(name);
            }
        }
        for bin in bins {
            if bin["required-features"] != () { throw "Cargo importer does not yet support required-features binaries"; }
            let name = rust_name(bin["name"]);
            let path = if bin["path"] != () { bin["path"] }
                else if bin["name"] == manifest["package"]["name"] && path_exists(rust_path(directory, "src/main.rs")) { "src/main.rs" }
                else if path_exists(rust_path(directory, `src/bin/${bin["name"]}.rs`)) { `src/bin/${bin["name"]}.rs` }
                else { `src/bin/${bin["name"]}/main.rs` };
            manifest["forge-bin-name"] = bin["name"];
            let target = if lib != () && name == lib["name"] { name + "_bin" } else { name };
            used = cargo_emit(target, "binary", rust_path(directory, path), directory, manifest, deps, aliases, used);
        }
    }
}

fn cargo_member_manifests(ctx) {
    let root = toml_decode(ctx.read_file("Cargo.toml"));
    let workspace = if root["workspace"] == () { #{} } else { root["workspace"] };
    let excluded = [];
    if workspace["exclude"] != () {
        for member in workspace["exclude"] { excluded += ctx.glob(rust_path(member, "Cargo.toml")); }
    }
    let paths = if root["package"] == () { [] } else { ["Cargo.toml"] };
    if workspace["members"] != () {
        for member in workspace["members"] { paths += ctx.glob(rust_path(member, "Cargo.toml")); }
    }
    let manifests = [];
    for path in paths {
        if excluded.contains(path) { continue; }
        manifests.push(cargo_inherit(toml_decode(ctx.read_file(path)), workspace));
    }
    manifests
}

fn rust_workspace(config) {
    let mode = rust_mode(config);
    if mode == "native" { return; }
    if !path_exists("Cargo.toml") { throw "cargo mode requires Cargo.toml"; }
    if !path_exists("Cargo.lock") { throw "cargo mode requires an explicit Cargo.lock; Forge does not run Cargo or solve Cargo versions"; }
    let root = toml_decode(read_file("Cargo.toml"));
    let lock = toml_decode(read_file("Cargo.lock"));
    let identities = #{};
    if lock["package"] != () {
        for pkg in lock["package"] {
            let source = pkg["source"];
            if source == () { continue; }
            let identity = `${pkg["name"]}@${pkg["version"]}`;
            if identities.contains(identity) { throw `Cargo lock source identity collision: ${identity}`; }
            identities[identity] = source;
            let deps = if pkg["dependencies"] == () { [] } else { pkg["dependencies"] };
            if source == "registry+https://github.com/rust-lang/crates.io-index" || source == "sparse+https://index.crates.io/" {
                dependency(pkg["name"], pkg["version"], `https://crates.io/api/v1/crates/${pkg["name"]}/${pkg["version"]}/download`, pkg["checksum"], deps);
            } else if source.starts_with("git+") {
                let parts = source.sub_string(4).split("#");
                if parts.len() != 2 || parts[1] == "" { throw `Cargo git source requires a locked revision: ${source}`; }
                let url = parts[0].split("?")[0];
                dependency_git(pkg["name"], pkg["version"], url, parts[1], deps);
            } else {
                throw `unsupported Cargo lock source: ${source}; only default crates.io and pinned git sources are supported`;
            }
        }
    }
    imported_lock("Cargo.lock");
    if !path_exists("FORGE.toml") && !path_exists("FORGE.rhai") {
        cargo_import_targets(root, lock);
    }
}
