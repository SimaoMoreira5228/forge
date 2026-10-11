fn code_lines(text) { 
	let out = [];
	let in_comment = false;
	for raw in text.split("\n") {
		let t = raw;
		t.trim();
		if in_comment {
			let end = t.index_of("*/");
			if end < 0 {
				continue;
			}
			t = t.sub_string(end + 2, t.len() - end - 2);
			t.trim();
			in_comment = false;
		}
		let again = true;
		while again {
			again = false;
			let open = t.index_of("/*");
			let line = t.index_of("//");
			if open >= 0 && (line < 0 || open < line) {
				let tail = t.sub_string(open + 2, t.len() - open - 2);
				let end = tail.index_of("*/");
				if end < 0 {
					t = t.sub_string(0, open);
					t.trim();
					in_comment = true;
					break;
				}
				t = t.sub_string(0, open) + tail.sub_string(end + 2, tail.len() - end - 2);
				t.trim();
				again = t != "";
			} else if line >= 0 {
				t = t.sub_string(0, line);
				t.trim();
			}
		}
		if t == "" {
			continue;
		}
		out.push(t);
	}
	out
}

fn first_quoted(line) {
	let first = line.index_of(`"`);
	if first < 0 {
		return "";
	}
	let rest = line.sub_string(first + 1, line.len() - first - 1);
	let end = rest.index_of(`"`);
	if end < 0 {
		return "";
	}
	rest.sub_string(0, end)
}

fn last_quoted(line) {
	let t = line;
	t.trim();
	if t.starts_with("import ") {
		t = t.sub_string(7, t.len() - 7);
		t.trim();
	}
	first_quoted(t)
}

fn scan_imports(text) {
	let imports = [];
	if !text.contains("import") {
		return imports;
	}
	let in_block = false;
	for line in code_lines(text) {
		let t = line;
		t.trim();
		if t == "import (" {
			in_block = true;
			continue;
		}
		if in_block && t == ")" {
			in_block = false;
			continue;
		}
		let candidate = "";
		if t.starts_with("import ") {
			candidate = last_quoted(t);
		} else if in_block {
			candidate = last_quoted(t);
		}
		if candidate == "" || imports.contains(candidate) {
			continue;
		}
		imports.push(candidate);
	}
	imports
}

fn has_cgo_import(text) {
	if !text.contains(`"C"`) {
		return false;
	} for line in code_lines(text) {
		let t = line;
		t.trim();
		if t.starts_with("import ") && last_quoted(t) == "C" {
			return true;
		}
		if t == `"C"` {
			return true;
		}
	}
	false
}

fn check_cgo_directives(text) {
	for line in text.split("\n") {
		let t = line;
		t.trim();
		if t.starts_with("#cgo ") {
			throw `unsupported #cgo directive: ${t}`;
		}
	}
}

fn package_clause(text) {
	for line in code_lines(text) {
		let t = line;
		t.trim();
		if t.starts_with("package ") {
			let clause = t.sub_string(8, t.len() - 8);
			clause.trim();
			return clause;
		}
	}
	""
}

fn test_funcs(text) {
	let found = [];
	for line in code_lines(text) {
		let t = line;
		t.trim();
		if !t.starts_with("func ") {
			continue;
		}
		let rest = t.sub_string(5, t.len() - 5);
		for kind in ["Test", "Benchmark", "Example"] {
			if rest.starts_with(kind) {
				let tail = rest.sub_string(kind.len(), rest.len() - kind.len());
				if tail.len() == 0 {
					continue;
				}
				let next = tail.sub_string(0, 1);
				if next == "(" {
					continue;
				}
				if "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".contains(next) {
					found.push(#{ kind: kind, name: `${kind}${tail.split("(")[0]}` });
				}
			}
		}
	}
	found
}

fn experiment_enabled(ctx, name) {
	if name == "regabiwrappers" || name == "regabiargs" {
		return ["amd64", "arm64", "loong64", "ppc64", "ppc64le", "riscv64", "s390x"].contains(goarch_of(ctx.platform_arch));
	}
	if name == "dwarf5" {
		let goos = goos_of(ctx.platform_os);
		return goos != "darwin" && goos != "ios" && goos != "aix";
	}
	if ["randomizedheapbase64", "greenteagc", "jsonv2", "sizespecializedmalloc"].contains(name) {
		return true;
	}
	false
}

fn build_tag_value(ctx, cgo_on, tag) {
	if tag == "ignore" {
		return false;
	}
	if tag == "cgo" {
		return cgo_on;
	}
	if tag == "gc" {
		return ctx.compiler != "gccgo";
	}
	if tag == "unix" {
		return goos_of(ctx.platform_os) != "windows";
	}
	if tag == "windows" || tag == "linux" || tag == "darwin" {
		return goos_of(ctx.platform_os) == tag;
	}
	if [
		"386",
		"amd64",
		"amd64p32",
		"arm",
		"arm64",
		"loong64",
		"mips",
		"mipsle",
		"mips64",
		"mips64le",
		"ppc64",
		"ppc64le",
		"riscv",
		"riscv64",
		"s390x",
		"sparc64",
		"wasm"
	].contains(tag) {
		return goarch_of(ctx.platform_arch) == tag;
	}
	if tag.starts_with("goexperiment.") {
		return experiment_enabled(ctx, tag.sub_string(13, tag.len() - 13));
	}
	if tag.starts_with("go1.") {
		return true;
	}
	false
}

fn build_expr(ctx, cgo_on, expr, level) {
	if level == 0 {
		let parts = [];
		let depth = 0;
		let start = 0;
		let index = 0;
		for ch in expr.chars() {
			if ch == "(" {
				depth += 1;
			} else if ch == ")" {
				depth -= 1;
			}
			if depth == 0 && ch == "|" && index + 1 < expr.len() && expr.sub_string(index + 1, 1) == "|" {
				parts.push(expr.sub_string(start, index - start));
				start = index + 2;
			}
			index += 1;
		}
		parts.push(expr.sub_string(start, expr.len() - start));
		if parts.len() < 2 {
			return build_expr(ctx, cgo_on, expr, 1);
		}
		for part in parts {
			if build_expr(ctx, cgo_on, part, 1) {
				return true;
			}
		}
		return false;
	}
	if level == 1 {
		let parts = [];
		let depth = 0;
		let start = 0;
		let index = 0;
		for ch in expr.chars() {
			if ch == "(" {
				depth += 1;
			} else if ch == ")" {
				depth -= 1;
			}
			if depth == 0 && ch == "&" && index + 1 < expr.len() && expr.sub_string(index + 1, 1) == "&" {
				parts.push(expr.sub_string(start, index - start));
				start = index + 2;
			}
			index += 1;
		}
		parts.push(expr.sub_string(start, expr.len() - start));
		if parts.len() < 2 {
			return build_expr(ctx, cgo_on, expr, 2);
		}
		for part in parts {
			if !build_expr(ctx, cgo_on, part, 2) {
				return false;
			}
		}
		return true;
	}
	let t = expr;
	t.trim();
	if t.starts_with("!") {
		return !build_expr(ctx, cgo_on, t.sub_string(1, t.len() - 1), 2);
	}
	if t.starts_with("(") && t.ends_with(")") {
		return build_expr(ctx, cgo_on, t.sub_string(1, t.len() - 2), 0);
	}
	build_tag_value(ctx, cgo_on, t)
}

fn build_lines_match(ctx, cgo_on, text) {
	if !text.contains("//go:build") && !text.contains("// +build") {
		return true;
	}
	let go_lines = [];
	let plus_lines = [];
	for line in text.split("\n") {
		let t = line;
		t.trim();
		if t.starts_with("//go:build ") {
			go_lines.push(t.sub_string(11, t.len() - 11));
		} else if t.starts_with("// +build ") {
			plus_lines.push(t.sub_string(10, t.len() - 10));
		}
	}
	if go_lines.len() > 0 {
		let result = true;
		for expr in go_lines {
			if !build_expr(ctx, cgo_on, expr, 0) {
				result = false;
			}
		}
		return result;
	}
	if plus_lines.len() == 0 {
		return true;
	}
	for line in plus_lines {
		let comma = true;
		for group in line.split(",") {
			let space = false;
			for term in group.split(" ") {
				let word = term;
				word.trim();
				if word == "" {
					continue;
				}
				if build_expr(ctx, cgo_on, word, 0) {
					space = true;
				}
			}
			if !space {
				comma = false;
			}
		}
		if comma {
			return true;
		}
	}
	false
}

fn basename(path) {
	let flat = path.split("/");
	flat[flat.len() - 1]
}

fn dirname(path) {
	let flat = path.split("/");
	if flat.len() < 2 {
		return "";
	}
	let out = flat[0];
	let i = 1;
	while i < flat.len() - 1 {
		out = out + `/${flat[i]}`;
		i += 1;
	}
	out
}

fn name_excluded(ctx, filename) {
	let stem = basename(filename);
	if stem.ends_with(".go") {
		stem = stem.sub_string(0, stem.len() - 3);
	} else if stem.ends_with(".c") {
		stem = stem.sub_string(0, stem.len() - 2);
	} else if stem.ends_with(".S") {
		stem = stem.sub_string(0, stem.len() - 2);
	} else if stem.ends_with(".s") {
		stem = stem.sub_string(0, stem.len() - 2);
	} else if stem.ends_with(".h") {
		stem = stem.sub_string(0, stem.len() - 2);
	}
	let gooses = [
		"aix",
		"android",
		"darwin",
		"dragonfly",
		"freebsd",
		"hurd",
		"illumos",
		"ios",
		"js",
		"linux",
		"netbsd",
		"openbsd",
		"plan9",
		"solaris",
		"wasip1",
		"windows",
		"zos"
	];
	let goarches = [
		"386",
		"amd64",
		"amd64p32",
		"arm",
		"arm64",
		"loong64",
		"mips",
		"mipsle",
		"mips64",
		"mips64le",
		"ppc64",
		"ppc64le",
		"riscv",
		"riscv64",
		"s390x",
		"sparc64",
		"wasm"
	];
	let parts = stem.split("_");
	let n = parts.len();
	if n > 1 && goarches.contains(parts[n - 1]) {
		if parts[n - 1] != goarch_of(ctx.platform_arch) {
			return true;
		}
		if n > 2 && gooses.contains(parts[n - 2]) && parts[n - 2] != goos_of(ctx.platform_os) {
			return true;
		}
		return false;
	}
	if n > 1 && gooses.contains(parts[n - 1]) {
		return parts[n - 1] != goos_of(ctx.platform_os);
	}
	false
}

fn src_kept(ctx, cgo_on, filename, text) {
	if filename.ends_with("_test.go") {
		return false;
	}
	if name_excluded(ctx, filename) {
		return false;
	}
	build_lines_match(ctx, cgo_on, text)
}
