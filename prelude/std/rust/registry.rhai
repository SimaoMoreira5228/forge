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

fn rust_registry_next(text) {
    let version = rust_registry_version(text);
    version[2] += 1;
    rust_registry_version_text(version)
}

fn rust_registry_clause(clause, req) {
    let op = "^";
    for prefix in [">=", "<=", "==", ">", "<", "=", "^", "~", "*"] {
        if clause.starts_with(prefix) { op = prefix; clause = clause.sub_string(prefix.len()); break; }
    }
    clause.trim();
    if op == "*" { return #{ min: "", max: "" }; }
    if clause == "" { throw `unsupported Rust requirement: ${req}`; }
    let version = ();
    try { version = rust_registry_version(clause); } catch (error) { throw `unsupported Rust requirement: ${req}; ${error}`; }
    let text = rust_registry_version_text(version);
    if op == "<" { return #{ min: "", max: text }; }
    if op == "<=" { return #{ min: "", max: rust_registry_next(text) }; }
    if op == ">" { return #{ min: rust_registry_next(text), max: "" }; }
    if op == "=" || op == "==" { return #{ min: text, max: rust_registry_next(text) }; }
    if op == ">=" { return #{ min: text, max: "" }; }
    if op == "~" {
        let bump = if clause.split(".").len() > 1 { 1 } else { 0 };
        version[bump] += 1;
        for i in (bump + 1)..3 { version[i] = 0; }
        return #{ min: text, max: rust_registry_version_text(version) };
    }
    if op == "^" {
        let last = clause.split(".").len() - 1;
        let bump = 0;
        while bump < last && version[bump] == 0 { bump += 1; }
        version[bump] += 1;
        for i in (bump + 1)..3 { version[i] = 0; }
        return #{ min: text, max: rust_registry_version_text(version) };
    }
    throw `unsupported Rust requirement: ${req}`;
}

fn rust_registry_requirement(req) {
    if req.type_of() != "string" { throw `unsupported Rust requirement: ${req}`; }
    req.trim();
    let bounds = #{ min: "", max: "" };
    if req == "" { return bounds; }
    for clause in req.split(",") {
        clause.trim();
        let interval = rust_registry_clause(clause, req);
        if interval.min != "" && (bounds.min == "" || rust_registry_before(bounds.min, interval.min)) { bounds.min = interval.min; }
        if interval.max != "" && (bounds.max == "" || rust_registry_before(interval.max, bounds.max)) { bounds.max = interval.max; }
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

fn rust_registry_config_url(registry) { `${registry}/config.json` }

fn rust_registry_sparse_url(name, registry) {
    let lower = name.to_lower();
    `${registry}/${rust_registry_prefix(lower)}/${lower}`
}

fn rust_registry_config(text) {
    let config = ();
    try { config = json_decode(text); }
    catch (error) { throw `config.json: ${error}`; }
    if config.type_of() != "map" { throw "config.json must be an object"; }
    if config.contains("auth-required") && config["auth-required"].type_of() != "bool" { throw "config.json auth-required must be a boolean"; }
    if config["auth-required"] == true { throw "unsupported config.json authentication (auth-required)"; }
    try { rust_registry_download_url(config.dl, "crate", "1.0.0", "checksum"); }
    catch (error) { throw `config.json dl: ${error}`; }
    config.dl
}

fn rust_registry_index(name, registry) {
    let url = rust_registry_sparse_url(name, registry);
    let cached = scratch_get(`rust-registry-index:${url}`);
    if cached != () { return json_decode(cached); }
    let entries = rust_registry_entries(name, fetch(url));
    scratch_put(`rust-registry-index:${url}`, json_encode(entries));
    entries
}

fn rust_registry_entries(name, text) {
    let entries = [];
    for line in text.split("\n") {
        line.trim();
        if line == "" { continue; }
        let entry = json_decode(line);
        if entry.type_of() != "map" { throw `invalid sparse index entry for ${name}`; }
        entries.push(entry);
    }
    entries
}

fn rust_registry_download_base(registry) {
    let cached = scratch_get(`rust-registry-config:${registry}`);
    if cached != () { return cached; }
    let base = rust_registry_config(fetch(rust_registry_config_url(registry)));
    scratch_put(`rust-registry-config:${registry}`, base);
    base
}

fn rust_registry_entry(entries, version) {
    for entry in entries {
        if entry.vers == version && rust_registry_release(entry) { return entry; }
    }
    ()
}

fn rust_registry_choice(name, registry, min, max) {
    let cache = `rust-registry-choice:${name}|${registry}|${min}|${max}`;
    let cached = scratch_get(cache);
    if cached != () { return cached; }
    let best = rust_registry_choose(rust_registry_index(name, registry), min, max);
    if best == () { return ""; }
    scratch_put(cache, best.vers);
    best.vers
}

fn rust_registry_pinned(name, selected, request) {
    if !selected.contains(name) { return ""; }
    let pinned = selected[name];
    if rust_registry_within(pinned, request.min, request.max) { pinned } else { "" }
}

fn rust_registry_optional(entry) {
    if entry.deps.type_of() != "array" { return false; }
    for dep in entry.deps {
        if dep.type_of() == "map" && dep.optional == true { return true; }
    }
    false
}

fn rust_registry_requested(request) {
    let features = [];
    for feature in request.features { if !features.contains(feature) { features.push(feature); } }
    if request["default"] && !features.contains("default") { features.push("default"); }
    features.sort();
    features
}

fn rust_registry_expand(entry, table, features, active) {
    let index = 0;
    while index < features.len() {
        let feature = features[index];
        index += 1;
        let members = table[feature];
        if members == () { continue; }
        for member in members {
            if member.starts_with("dep:") {
                let depkey = member.sub_string(4);
                if !active.contains(depkey) { active.push(depkey); }
            } else if member.contains("/") {
                let parts = member.split("/");
                let depkey = parts[0];
                let weak = depkey.ends_with("?");
                if weak { depkey = depkey.sub_string(0, depkey.len() - 1); }
                let dep = rust_registry_find_dep(entry, depkey);
                let already = active.contains(depkey) || (dep != () && dep.optional != true);
                if (!weak || already) && !active.contains(depkey) { active.push(depkey); }
            } else if table[member] != () {
                if !features.contains(member) { features.push(member); }
            } else {
                let dep = rust_registry_find_dep(entry, member);
                if dep != () && dep.optional == true && !active.contains(member) { active.push(member); }
            }
        }
    }
    #{ features: features, active: active }
}

fn rust_registry_activated(entry, request) {
    rust_registry_expand(entry, rust_registry_feature_table(entry), rust_registry_requested(request), []).active
}

fn rust_registry_child(dep, table, request, closure) {
    if dep.type_of() != "map" { throw `invalid sparse index dependency`; }
    if dep.kind == "dev" { return (); }
    let depkey = dep.name;
    if dep.optional == true && !closure.active.contains(depkey) { return (); }
    let child = if dep["package"] == () { depkey } else { dep["package"] };
    rust_name(child);
    child = child.to_lower();
    if dep.registry != () && rust_registry_index_url(dep.registry) != request.registry { throw `unsupported cross-registry dependency: ${dep.registry}`; }
    let range = rust_registry_requirement(dep.req);
    let forwarded = rust_registry_forwarded(table, closure.features, depkey);
    if dep.features != () {
        for feature in dep.features {
            if !forwarded.contains(feature) { forwarded.push(feature); }
        }
    }
    #{ name: child, min: range.min, max: range.max, "default": dep.default_features != false, features: forwarded }
}

fn rust_registry_feature_key(features) {
    let key = "";
    for feature in features { if key != "" { key += ","; } key += feature; }
    key
}

fn rust_registry_plan(name, registry, selected, request) {
    let version = rust_registry_pinned(name, selected, request);
    if version == "" { version = rust_registry_choice(name, registry, request.min, request.max); }
    if version == "" { return (); }
    let requested = rust_registry_requested(request);
    let cache = `rust-registry-plan:${name}@${version}|${rust_registry_feature_key(requested)}|${registry}`;
    let cached = scratch_get(cache);
    if cached != () { return json_decode(cached); }
    let entry = rust_registry_entry(rust_registry_index(name, registry), version);
    if entry == () { throw `registry ${registry} has no release ${name}@${version}`; }
    let table = rust_registry_feature_table(entry);
    let closure = rust_registry_expand(entry, table, requested, []);
    let children = [];
    if entry.deps.type_of() != "array" { throw `invalid sparse index dependencies`; }
    for dep in entry.deps {
        let child = rust_registry_child(dep, table, request, closure);
        if child != () { children.push(child); }
    }
    let plan = #{ features: closure.features, active: closure.active, children: children };
    scratch_put(cache, json_encode(plan));
    plan
}

fn rust_registry_candidates(pending, selected, conflict) {
    let registries = #{};
    let downloads = #{};
    let requests = #{};

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
                if downloads[request.registry] == () { downloads[request.registry] = rust_registry_download_base(request.registry); }
                let plan = rust_registry_plan(name, request.registry, selected, request);
                if plan == () { continue; }
                for child in plan.children {
                    let merged = rust_registry_merge_request(
                        requests[child.name], child.min, child.max, child["default"], child.features, request.registry);
                    if merged[0] { stable = false; }
                    requests[child.name] = merged[1];
                }
            } catch (error) { throw `Rust registry ${request.registry}, crate ${name}: ${error}`; }
        }
    }

    let pinned = selected.len() > 0;
    let narrowed = pinned || conflict == "";
    for name in requests.keys() {
        let request = requests[name];
        try {
            for entry in rust_registry_index(name, request.registry) {
                if !rust_registry_release(entry) { continue; }
                if narrowed && !rust_registry_within(entry.vers, request.min, request.max) { continue; }
                let version = entry.vers;
                let active = if rust_registry_optional(entry) { rust_registry_activated(entry, request) } else { [] };
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
