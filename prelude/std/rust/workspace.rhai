if path_exists("Cargo.lock") {
    let text = read_file("Cargo.lock");
    let lock = toml_decode(text);
    let needed = [];
    for manifest_path in glob("crates/**/Cargo.toml") {
        let manifest = toml_decode(read_file(manifest_path));
        for section_name in ["dependencies", "build-dependencies"] {
            let section = manifest[section_name];
            if section == () { continue; }
            for name in section.keys() {
                let spec = section[name];
                let package_name = if spec.type_of() == "map" && spec["package"] != () { spec["package"] } else { name };
                if !needed.contains(package_name) { needed.push(package_name); }
            }
        }
    }
    let index = 0;
    while index < needed.len() {
        let name = needed[index];
        index += 1;
        for pkg in lock["package"] {
            if pkg["name"] != name || pkg["dependencies"] == () { continue; }
            for dependency in pkg["dependencies"] {
                let dependency_name = if dependency.contains(" package ") {
                    dependency.split(" package ")[1].split(" ")[0]
                } else {
                    dependency.split(" ")[0]
                };
                if !needed.contains(dependency_name) { needed.push(dependency_name); }
            }
        }
    }
    for pkg in lock["package"] {
        if !needed.contains(pkg["name"]) || pkg["source"] == () {
            continue;
        }
        let deps = [];
        if pkg["dependencies"] != () {
            for dependency in pkg["dependencies"] {
                deps.push(dependency);
            }
        }
        let source = pkg["source"];
        if source.contains("registry+") {
            let url = `https://crates.io/api/v1/crates/${pkg["name"]}/${pkg["version"]}/download`;
            dependency(pkg["name"], pkg["version"], url, pkg["checksum"], deps);
        } else if source.contains("git+") {
            let parts = source.split("git+")[1].split("#");
            let url = parts[0].split("?")[0];
            dependency_git(pkg["name"], pkg["version"], url, parts[1], deps);
        }
    }
}
