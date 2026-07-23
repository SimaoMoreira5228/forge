fn fetch_cargo_lock(path) {
    let text = read_file(path);
    let lock = toml_decode(text);
    for pkg in lock["package"] {
        if pkg["checksum"] != () && pkg["checksum"] != "" && pkg["source"] != () && pkg["source"].contains("registry+") {
            let deps = [];
            if pkg["dependencies"] != () {
                for d in pkg["dependencies"] {
                    deps.push(d);
                }
            }
            let url = `https://crates.io/api/v1/crates/${pkg["name"]}/${pkg["version"]}/download`;
            dependency(pkg["name"], pkg["version"], url, pkg["checksum"], deps);
        }
    }
}
