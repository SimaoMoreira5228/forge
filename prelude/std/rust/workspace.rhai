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

fn rust_registry_http_url(url) {
    if url.type_of() != "string" || (!url.starts_with("https://") && !url.starts_with("http://")) { throw "expected HTTP(S) Rust registry URL"; }
    for ch in url.chars() {
        if ch <= ' ' || ch >= '\u007f' || "\\\"<>`{}#".contains(ch) { throw "unsupported or malformed Rust registry URL"; }
    }
    let authority = url.sub_string(if url.starts_with("https://") { 8 } else { 7 }).split("/")[0].split("?")[0];
    if authority.contains("@") { throw "unsupported Rust registry URL authentication"; }
    let parts = authority.split(":");
    if parts.len() > 2 || parts[0] == "" { throw "unsupported or malformed Rust registry host"; }
    for label in parts[0].split(".") {
        if label == "" || label.starts_with("-") || label.ends_with("-") { throw "malformed Rust registry host"; }
        for ch in label.to_lower().chars() {
            if !"abcdefghijklmnopqrstuvwxyz0123456789-".contains(ch) { throw "unsupported Rust registry host"; }
        }
    }
    if parts.len() == 2 {
        if parts[1] == "" || parts[1].len() > 5 { throw "malformed Rust registry port"; }
        for ch in parts[1].chars() {
            if !"0123456789".contains(ch) { throw "malformed Rust registry port"; }
        }
        if parse_int(parts[1]) > 65535 { throw "malformed Rust registry port"; }
    }
    let escapes = url.split("%");
    for i in 1..escapes.len() {
        if escapes[i].len() < 2 { throw "malformed Rust registry URL escape"; }
        for ch in escapes[i].sub_string(0, 2).to_lower().chars() {
            if !"0123456789abcdef".contains(ch) { throw "malformed Rust registry URL escape"; }
        }
    }
    url
}

fn rust_registry_prefix(name) {
    if name.len() <= 2 { name.len().to_string() }
    else if name.len() == 3 { "3/" + name.sub_string(0, 1) }
    else { name.sub_string(0, 2) + "/" + name.sub_string(2, 2) }
}

fn rust_registry_download_url(dl, name, version, checksum) {
    if dl.type_of() != "string" || dl == "" { throw "config.json requires a non-empty dl string"; }
    let template = dl.contains("{") || dl.contains("}");
    if template {
        dl.replace("{crate}", name);
        dl.replace("{version}", version);
        dl.replace("{prefix}", rust_registry_prefix(name));
        dl.replace("{lowerprefix}", rust_registry_prefix(name.to_lower()));
        dl.replace("{sha256-checksum}", checksum);
    } else {
        rust_registry_http_url(dl);
        if dl.contains("?") { throw "unsupported config.json dl base URL query"; }
        while dl.ends_with("/") { dl = dl.sub_string(0, dl.len() - 1); }
        dl += `/${name}/${version}/download`;
    }
    rust_registry_http_url(dl)
}

fn rust_registry_index_url(registry) {
    if registry == () || registry == "crates.io" { return "https://index.crates.io"; }
    if registry.type_of() != "string" { throw `invalid Rust registry URL: ${registry}`; }
    rust_registry_http_url(registry);
    if registry.contains("?") || registry.contains("#") { throw `unsupported Rust registry URL: ${registry}`; }
    while registry.ends_with("/") { registry = registry.sub_string(0, registry.len() - 1); }
    registry
}

fn rust_registry_version(version) {
    if version.type_of() != "string" { throw `expected version string: ${version}`; }
    let parts = version.split(".");
    if parts.len() < 1 || parts.len() > 3 { throw `unsupported Rust version: ${version}`; }
    let numbers = [];
    for part in parts {
        if part == "" || (part.len() > 1 && part.starts_with("0")) { throw `unsupported Rust version: ${version}`; }
        for ch in part.chars() {
            if !"0123456789".contains(ch) { throw `unsupported Rust version: ${version}`; }
        }
        numbers.push(parse_int(part));
    }
    while numbers.len() < 3 { numbers.push(0); }
    numbers
}

fn rust_registry_version_text(version) { `${version[0]}.${version[1]}.${version[2]}` }

fn rust_registry_before(left, right) {
    let a = rust_registry_version(left);
    let b = rust_registry_version(right);
    for i in 0..3 {
        if a[i] != b[i] { return a[i] < b[i]; }
    }
    false
}

fn rust_registry_range(range) {
    if range.type_of() != "map" { throw "native dependency range must be a min/max table"; }
    for key in range.keys() {
        if !["min", "max"].contains(key) { throw `unknown native range key: ${key}`; }
    }
    let bounds = #{ min: "", max: "" };
    for key in bounds.keys() {
        if range[key] != () && range[key] != "" { bounds[key] = rust_registry_version_text(rust_registry_version(range[key])); }
    }
    if bounds.min != "" && bounds.max != "" && !rust_registry_before(bounds.min, bounds.max) { throw "native dependency range must have min < max"; }
    bounds
}

fn rust_registry_requirement(req) {
    if req.type_of() != "string" { throw `unsupported Rust requirement: ${req}`; }
    req.trim();
    let bounds = #{ min: "", max: "" };
    if req == "" { return bounds; }
    for clause in req.split(",") {
        clause.trim();
        let op = "^";
        for prefix in [">=", "<", "^", "~"] {
            if clause.starts_with(prefix) { op = prefix; clause = clause.sub_string(prefix.len()); break; }
        }
        clause.trim();
        let version = ();
        try { version = rust_registry_version(clause); } catch (error) { throw `unsupported Rust requirement: ${req}; ${error}`; }
        let minimum = if op == "<" { "" } else { rust_registry_version_text(version) };
        let maximum = if op == "<" { rust_registry_version_text(version) } else { "" };
        if op == "^" || op == "~" {
            let last = clause.split(".").len() - 1;
            let bump = 0;
            if op == "~" { bump = if last == 0 { 0 } else { 1 }; }
            else { while bump < last && version[bump] == 0 { bump += 1; } }
            version[bump] += 1;
            for i in (bump + 1)..3 { version[i] = 0; }
            maximum = rust_registry_version_text(version);
        }
        if minimum != "" && (bounds.min == "" || rust_registry_before(bounds.min, minimum)) { bounds.min = minimum; }
        if maximum != "" && (bounds.max == "" || rust_registry_before(maximum, bounds.max)) { bounds.max = maximum; }
    }
    rust_registry_range(bounds)
}

fn rust_registry_within(version, min, max) {
    if min != "" && rust_registry_before(version, min) { return false; }
    if max != "" && !rust_registry_before(version, max) { return false; }
    true
}

fn rust_registry_release(entry) {
    if entry.yanked == true { return false; }
    let version = entry.vers;
    if version.type_of() != "string" { return false; }
    if version.contains("-") || version.contains("+") { return false; }
    rust_registry_version(version);
    true
}

fn rust_registry_choose(entries, min, max) {
    let best = ();
    for entry in entries {
        if !rust_registry_release(entry) { continue; }
        if !rust_registry_within(entry.vers, min, max) { continue; }
        if best == () || rust_registry_before(best.vers, entry.vers) { best = entry; }
    }
    best
}

fn rust_registry_feature_table(entry) {
    let table = #{};
    for source in [entry.features, entry.features2] {
        if source == () { continue; }
        if source.type_of() != "map" { throw "invalid sparse index features"; }
        for key in source.keys() {
            let members = source[key];
            if members.type_of() != "array" { throw `invalid sparse index feature ${key}`; }
            if table[key] == () { table[key] = []; }
            for member in members {
                if member.type_of() != "string" { throw `invalid sparse index feature member for ${key}`; }
                if !table[key].contains(member) { table[key].push(member); }
            }
        }
    }
    table
}

fn rust_registry_find_dep(entry, key) {
    if entry.deps.type_of() != "array" { return (); }
    for dep in entry.deps {
        if dep.type_of() == "map" && dep.name == key { return dep; }
    }
    ()
}

fn rust_registry_forwarded(table, features, depkey) {
    let forwarded = [];
    for feature in features {
        let members = table[feature];
        if members == () { continue; }
        for member in members {
            if !member.contains("/") { continue; }
            let parts = member.split("/");
            let key = parts[0];
            if key.ends_with("?") { key = key.sub_string(0, key.len() - 1); }
            if key != depkey { continue; }
            if !forwarded.contains(parts[1]) { forwarded.push(parts[1]); }
        }
    }
    forwarded
}

fn rust_registry_config(registry, downloads) {
    if downloads[registry] != () { return downloads[registry]; }
    let config = ();
    try { config = json_decode(http_get(`${registry}/config.json`)); }
    catch (error) { throw `config.json: ${error}`; }
    if config.type_of() != "map" { throw "config.json must be an object"; }
    if config.contains("auth-required") && config["auth-required"].type_of() != "bool" { throw "config.json auth-required must be a boolean"; }
    if config["auth-required"] == true { throw "unsupported config.json authentication (auth-required)"; }
    try { rust_registry_download_url(config.dl, "crate", "1.0.0", "checksum"); }
    catch (error) { throw `config.json dl: ${error}`; }
    config.dl
}

fn rust_registry_index(name, registry) {
    let lower = name.to_lower();
    let prefix = rust_registry_prefix(lower);
    let entries = [];
    for line in http_get(`${registry}/${prefix}/${lower}`).split("\n") {
        line.trim();
        if line == "" { continue; }
        let entry = json_decode(line);
        if entry.type_of() != "map" { throw `invalid sparse index entry for ${name}`; }
        entries.push(entry);
    }
    entries
}

fn rust_registry_candidates(pending) {
    let indexes = #{};
    let registries = #{};
    let downloads = #{};
    let requests = #{};
    let enabled = #{};
    let activated = #{};

    for request in pending {
        rust_name(request.name);
        requests[request.name] = #{
            min: request.min,
            max: request.max,
            "default": if request["default"] == () { true } else { request["default"] },
            features: if request.features == () { [] } else { request.features },
            registry: request.registry,
        };
    }

    let stable = false;
    while !stable {
        stable = true;
        for name in requests.keys() {
            let request = requests[name];
            try {
                if registries[name] != () && registries[name] != request.registry { throw `conflicting registries`; }
                registries[name] = request.registry;
                downloads[request.registry] = rust_registry_config(request.registry, downloads);
                if indexes[name] == () {
                    indexes[name] = rust_registry_index(name, request.registry);
                    stable = false;
                }

                let features = if enabled[name] == () { [] } else { enabled[name] };
                for feature in request.features {
                    if !features.contains(feature) {
                        features.push(feature);
                        stable = false;
                    }
                }
                if request["default"] == true && !features.contains("default") {
                    features.push("default");
                    stable = false;
                }
                let active = if activated[name] == () { [] } else { activated[name] };
                let chosen = rust_registry_choose(indexes[name], request.min, request.max);
                if chosen == () { enabled[name] = features; activated[name] = active; continue; }
                let table = rust_registry_feature_table(chosen);

                let index = 0;
                while index < features.len() {
                    let feature = features[index];
                    index += 1;
                    let members = table[feature];
                    if members == () { continue; }
                    for member in members {
                        if member.starts_with("dep:") {
                            let depkey = member.sub_string(4);
                            if !active.contains(depkey) { active.push(depkey); stable = false; }
                        } else if member.contains("/") {
                            let parts = member.split("/");
                            let depkey = parts[0];
                            let weak = depkey.ends_with("?");
                            if weak { depkey = depkey.sub_string(0, depkey.len() - 1); }
                            let dep = rust_registry_find_dep(chosen, depkey);
                            let already = active.contains(depkey) || (dep != () && dep.optional != true);
                            if !weak || already {
                                if !active.contains(depkey) { active.push(depkey); stable = false; }
                            }
                        } else if table[member] != () {
                            if !features.contains(member) { features.push(member); stable = false; }
                        } else {
                            let dep = rust_registry_find_dep(chosen, member);
                            if dep != () && dep.optional == true && !active.contains(member) {
                                active.push(member);
                                stable = false;
                            }
                        }
                    }
                }
                enabled[name] = features;
                activated[name] = active;

                if chosen.deps.type_of() != "array" { throw `invalid sparse index dependencies`; }
                for dep in chosen.deps {
                    if dep.type_of() != "map" { throw `invalid sparse index dependency`; }
                    if dep.kind == "dev" { continue; }
                    let depkey = dep.name;
                    if dep.optional == true && !active.contains(depkey) { continue; }
                    let child = if dep["package"] == () { depkey } else { dep["package"] };
                    rust_name(child);
                    child = child.to_lower();
                    if dep.registry != () && rust_registry_index_url(dep.registry) != request.registry { throw `unsupported cross-registry dependency: ${dep.registry}`; }
                    let range = rust_registry_requirement(dep.req);
                    let dep_default = dep.default_features != false;
                    let forwarded = rust_registry_forwarded(table, features, depkey);
                    if dep.features != () {
                        for feature in dep.features {
                            if !forwarded.contains(feature) { forwarded.push(feature); }
                        }
                    }
                    let merged = rust_registry_merge_request(requests[child], range.min, range.max, dep_default, forwarded, request.registry);
                    if merged[0] {
                        stable = false;
                    }
                    requests[child] = merged[1];
                }
            } catch (error) { throw `Rust registry ${request.registry}, crate ${name}: ${error}`; }
        }
    }

    let emitted = #{};
    for name in indexes.keys() {
        let request = requests[name];
        try {
            for entry in indexes[name] {
                if !rust_registry_release(entry) { continue; }
                if !rust_registry_within(entry.vers, request.min, request.max) { continue; }
                let version = entry.vers;
                let identity = `${name}@${version}`;
                if emitted.contains(identity) { continue; }
                let active = if activated[name] == () { [] } else { activated[name] };
                let deps = [];
                if entry.deps.type_of() != "array" { throw `invalid sparse index dependencies`; }
                for dep in entry.deps {
                    if dep.type_of() != "map" { throw `invalid sparse index dependency`; }
                    if dep.kind == "dev" { continue; }
                    let depkey = dep.name;
                    if dep.optional == true && !active.contains(depkey) { continue; }
                    let child = if dep["package"] == () { depkey } else { dep["package"] };
                    rust_name(child);
                    child = child.to_lower();
                    if dep.registry != () && rust_registry_index_url(dep.registry) != request.registry { throw `unsupported cross-registry dependency: ${dep.registry}`; }
                    let range = rust_registry_requirement(dep.req);
                    deps.push(#{ name: child, min: range.min, max: range.max });
                }
                let source = rust_registry_download_url(downloads[request.registry], name, version, entry.cksum);
                dependency_candidate(name, version, source, entry.cksum, deps);
                emitted[identity] = true;
            }
        } catch (error) { throw `Rust registry ${request.registry}, crate ${name}: ${error}`; }
    }
}

fn rust_registry_merge_request(existing, min, max, default_feature, forwarded, registry) {
    if existing == () {
        return [true, #{ min: min, max: max, "default": default_feature, features: forwarded, registry: registry }];
    }
    let changed = false;
    if min != "" && (existing.min == "" || rust_registry_before(existing.min, min)) { existing.min = min; changed = true; }
    if max != "" && (existing.max == "" || rust_registry_before(max, existing.max)) { existing.max = max; changed = true; }
    if default_feature && existing["default"] == false { existing["default"] = true; changed = true; }
    for feature in forwarded {
        if !existing.features.contains(feature) { existing.features.push(feature); changed = true; }
    }
    if existing.registry != registry { throw `conflicting registries`; }
    [changed, existing]
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
