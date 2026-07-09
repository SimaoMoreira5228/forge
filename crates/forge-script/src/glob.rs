use std::path::{Path, PathBuf};

pub fn glob_match(pattern: &str, path: &str) -> bool {
	let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
	let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
	match_segments(&pat, &segs)
}

fn match_segments(pat: &[&str], segs: &[&str]) -> bool {
	match pat.split_first() {
		None => segs.is_empty(),
		Some((&"**", rest)) => (0..=segs.len()).any(|skip| match_segments(rest, &segs[skip..])),
		Some((segment, rest)) => {
			let Some((head, tail)) = segs.split_first() else {
				return false;
			};
			segment_matches(segment, head) && match_segments(rest, tail)
		}
	}
}

fn segment_matches(pattern: &str, name: &str) -> bool {
	let p: Vec<char> = pattern.chars().collect();
	let n: Vec<char> = name.chars().collect();
	let (mut pi, mut ni) = (0usize, 0usize);
	let (mut star, mut mark) = (None::<usize>, 0usize);
	while ni < n.len() {
		if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
			pi += 1;
			ni += 1;
		} else if pi < p.len() && p[pi] == '*' {
			star = Some(pi);
			pi += 1;
			mark = ni;
		} else if let Some(s) = star {
			pi = s + 1;
			mark += 1;
			ni = mark;
		} else {
			return false;
		}
	}
	while pi < p.len() && p[pi] == '*' {
		pi += 1;
	}
	pi == p.len()
}

pub fn list_files(root: &Path) -> std::io::Result<Vec<String>> {
	let mut out = Vec::new();
	let mut stack = vec![PathBuf::from("")];
	while let Some(rel) = stack.pop() {
		let abs = if rel.as_os_str().is_empty() {
			root.to_path_buf()
		} else {
			root.join(&rel)
		};
		let Ok(entries) = std::fs::read_dir(&abs) else {
			continue;
		};
		for entry in entries.flatten() {
			let child_rel = if rel.as_os_str().is_empty() {
				PathBuf::from(entry.file_name())
			} else {
				rel.join(entry.file_name())
			};
			if entry.file_type().is_ok_and(|t| t.is_dir()) {
				stack.push(child_rel);
			} else {
				out.push(child_rel.to_string_lossy().into_owned());
			}
		}
	}
	out.sort();
	Ok(out)
}

pub fn expand_glob(root: &Path, pattern: &str) -> std::io::Result<Vec<PathBuf>> {
	Ok(list_files(root)?
		.into_iter()
		.filter(|rel| glob_match(pattern, rel))
		.map(PathBuf::from)
		.collect())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn single_star_stays_in_segment() {
		assert!(glob_match("lib/*.cpp", "lib/math.cpp"));
		assert!(!glob_match("lib/*.cpp", "lib/a/b.cpp"));
		assert!(glob_match("src/*.c", "src/main.c"));
	}

	#[test]
	fn double_star_spans_directories() {
		assert!(glob_match("src/**/*.c", "src/main.c"));
		assert!(glob_match("src/**/*.c", "src/deep/nested/x.c"));
		assert!(!glob_match("src/**/*.c", "other/main.c"));
		assert!(glob_match("**/*.md", "README.md"));
	}

	#[test]
	fn question_mark_and_exact() {
		assert!(glob_match("?.c", "a.c"));
		assert!(!glob_match("?.c", "ab.c"));
		assert!(glob_match("FORGE.toml", "FORGE.toml"));
	}

	#[test]
	fn segment_wildcard_basics() {
		assert!(segment_matches("*", "anything"));
		assert!(segment_matches("a*", "abc"));
		assert!(segment_matches("*c", "abc"));
		assert!(segment_matches("a*c", "abc"));
		assert!(!segment_matches("a*b", "acbbx"));
		assert!(segment_matches("a*?", "axb"));
	}

	#[test]
	fn expand_walks_tree_sorted() {
		let tmp = std::env::temp_dir().join(format!("forge-glob-test-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&tmp);
		std::fs::create_dir_all(tmp.join("lib/deep")).unwrap();
		std::fs::write(tmp.join("lib/b.cpp"), "").unwrap();
		std::fs::write(tmp.join("lib/deep/a.cpp"), "").unwrap();
		std::fs::write(tmp.join("lib/skip.txt"), "").unwrap();

		let hits = expand_glob(&tmp, "lib/**/*.cpp").unwrap();
		assert_eq!(hits, vec![PathBuf::from("lib/b.cpp"), PathBuf::from("lib/deep/a.cpp")]);
		let _ = std::fs::remove_dir_all(&tmp);
	}
}
