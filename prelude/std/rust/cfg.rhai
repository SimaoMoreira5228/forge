fn rust_cfg_matches(ctx, expression) {
    expression.trim();
    if !expression.starts_with("cfg(") { return expression == target_for(ctx); }
    rust_cfg_evaluate(ctx, expression)
}

fn rust_cfg_evaluate(ctx, expression) {
    expression.trim();
    let quoted = false;
    let depth = 0;
    let start = 0;
    let open = -1;
    let parts = [];
    for (ch, index) in expression.chars() {
        if ch == '"' { quoted = !quoted; }
        if quoted {
            if ch == '\\' { throw "unsupported escape in Rust cfg predicate"; }
            continue;
        }
        if ch == '(' {
            if depth == 0 { open = index; start = index + 1; }
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth < 0 || (depth == 0 && index != expression.len() - 1) { throw `malformed Rust cfg predicate: ${expression}`; }
        }
        if (ch == ',' && depth == 1) || (ch == ')' && depth == 0) {
            let part = expression.sub_string(start, index - start);
            part.trim();
            if part != "" { parts.push(part); }
            else if ch == ',' { throw `empty Rust cfg argument: ${expression}`; }
            start = index + 1;
        }
    }
    if quoted || depth != 0 { throw `unclosed Rust cfg predicate: ${expression}`; }
    if open >= 0 {
        let operator = expression.sub_string(0, open);
        operator.trim();
        if !["cfg", "all", "any", "not"].contains(operator) { throw `unsupported Rust cfg operator: ${operator}`; }
        if (operator == "cfg" || operator == "not") && parts.len() != 1 { throw `Rust cfg ${operator} requires one argument`; }
        let result = operator != "any";
        for part in parts {
            let value = rust_cfg_evaluate(ctx, part);
            if operator == "any" { result = result || value; }
            else if operator == "not" { result = !value; }
            else { result = result && value; }
        }
        return result;
    }
    let fields = expression.split("=");
    let name = fields[0];
    name.trim();
    if name == "" { throw "empty Rust cfg predicate"; }
    for ch in name.chars() {
        if !"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".contains(ch) { throw `invalid Rust cfg predicate: ${expression}`; }
    }
    if fields.len() == 1 {
        if name == "windows" { return ctx.platform_os == "windows"; }
        if name == "unix" { return ctx.platform_os != "windows"; }
        if name == "debug_assertions" { return ctx.profile.is_debug; }
        return false;
    }
    if fields.len() != 2 { throw `malformed Rust cfg value: ${expression}`; }
    let value = fields[1];
    value.trim();
    if value.len() < 2 || !value.starts_with("\"") || !value.ends_with("\"") { throw `Rust cfg value must be quoted: ${expression}`; }
    value = value.sub_string(1, value.len() - 2);
    if value.contains('"') { throw `malformed Rust cfg value: ${expression}`; }
    if name == "target_os" { return value == if ctx.platform_os == "darwin" { "macos" } else { ctx.platform_os }; }
    if name == "target_arch" { return value == ctx.platform_arch; }
    if name == "target_env" { return value == ctx.platform_abi; }
    if name == "target_vendor" { return value == if ctx.platform_os == "darwin" { "apple" } else if ctx.platform_os == "windows" { "pc" } else { "unknown" }; }
    if name == "target_endian" { return value == "little"; }
    if name == "target_pointer_width" { return value == if ["x86_64", "aarch64"].contains(ctx.platform_arch) { "64" } else { "32" }; }
    if name == "target_family" { return value == if ctx.platform_os == "windows" { "windows" } else { "unix" }; }
    false
}
