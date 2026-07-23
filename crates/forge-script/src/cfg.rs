pub struct PlatformFacts<'a> {
	pub os: &'a str,
	pub arch: &'a str,
	pub abi: &'a str,
	pub debug: bool,
}

pub fn host_triple(facts: &PlatformFacts<'_>) -> String {
	match facts.os {
		"linux" => format!("{}-unknown-linux-{}", facts.arch, facts.abi),
		"darwin" => format!("{}-apple-darwin", facts.arch),
		"windows" => format!("{}-pc-windows-msvc", facts.arch),
		other => format!("{}-unknown-{}", facts.arch, other),
	}
}

pub fn matches(expr: &str, facts: &PlatformFacts<'_>) -> bool {
	if !expr.starts_with("cfg(") {
		return expr == host_triple(facts);
	}
	evaluate(expr, facts)
}

fn evaluate(expr: &str, facts: &PlatformFacts<'_>) -> bool {
	let body: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
	let body = body
		.strip_prefix("cfg(")
		.and_then(|inner| inner.strip_suffix(')'))
		.unwrap_or(&body);
	if let Some(inner) = call(body, "any(") {
		return split_args(inner).into_iter().any(|part| evaluate(part, facts));
	}
	if let Some(inner) = call(body, "all(") {
		return split_args(inner).into_iter().all(|part| evaluate(part, facts));
	}
	if let Some(inner) = call(body, "not(") {
		return !evaluate(inner, facts);
	}
	predicate(body, facts)
}

fn call<'a>(body: &'a str, name: &str) -> Option<&'a str> {
	body.strip_prefix(name).and_then(|inner| inner.strip_suffix(')'))
}

fn split_args(text: &str) -> Vec<&str> {
	let mut parts = Vec::new();
	let mut depth = 0usize;
	let mut start = 0usize;
	for (index, ch) in text.char_indices() {
		match ch {
			'(' => depth += 1,
			')' => depth = depth.saturating_sub(1),
			',' if depth == 0 => {
				parts.push(&text[start..index]);
				start = index + 1;
			}
			_ => {}
		}
	}
	parts.push(&text[start..]);
	parts
}

fn predicate(predicate: &str, facts: &PlatformFacts<'_>) -> bool {
	match predicate {
		"windows" => return facts.os == "windows",
		"unix" => return facts.os != "windows",
		"debug_assertions" => return facts.debug,
		_ => {}
	}
	let Some((name, raw)) = predicate.split_once('=') else {
		return false;
	};
	let value = raw.trim_matches('"');
	match name {
		"target_os" => value == target_os(facts.os),
		"target_arch" => value == facts.arch,
		"target_env" => value == facts.abi,
		"target_vendor" => value == target_vendor(facts.os),
		"target_endian" => value == "little",
		"target_pointer_width" => value == pointer_width(facts.arch),
		"target_family" => (value == "unix" && facts.os != "windows") || (value == "windows" && facts.os == "windows"),
		_ => false,
	}
}

fn target_os(os: &str) -> &str {
	if os == "darwin" { "macos" } else { os }
}

fn target_vendor(os: &str) -> &str {
	match os {
		"darwin" => "apple",
		"windows" => "pc",
		_ => "unknown",
	}
}

fn pointer_width(arch: &str) -> &str {
	if arch == "x86_64" || arch == "aarch64" { "64" } else { "32" }
}

#[cfg(test)]
mod tests {
	use super::*;

	fn linux() -> PlatformFacts<'static> {
		PlatformFacts {
			os: "linux",
			arch: "x86_64",
			abi: "gnu",
			debug: true,
		}
	}

	#[test]
	fn windows_only_is_inactive_on_linux() {
		assert!(!matches("cfg(windows)", &linux()));
		assert!(matches("cfg(unix)", &linux()));
		assert!(matches("cfg(target_os = \"linux\")", &linux()));
		assert!(!matches("cfg(target_os = \"windows\")", &linux()));
	}

	#[test]
	fn nested_any_all_not() {
		let getrandom = "cfg(all(any(target_os = \"linux\", target_os = \"android\"), not(any(all(target_os = \"linux\", target_env = \"\"), getrandom_backend = \"custom\"))))";
		assert!(matches(getrandom, &linux()));
		assert!(!matches(
			"cfg(all(all(target_arch = \"aarch64\", target_endian = \"little\"), target_os = \"linux\"))",
			&linux()
		));
		assert!(matches(
			"cfg(any(target_arch = \"wasm32\", target_arch = \"x86_64\"))",
			&linux()
		));
		assert!(!matches(
			"cfg(any(target_arch = \"wasm32\", getrandom_backend = \"custom\"))",
			&linux()
		));
	}

	#[test]
	fn literal_triple_compares_to_host() {
		assert!(matches("x86_64-unknown-linux-gnu", &linux()));
		assert!(!matches("x86_64-pc-windows-msvc", &linux()));
	}
}
