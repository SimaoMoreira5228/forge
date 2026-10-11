use std::path::{Path, PathBuf};

use forge_diagnostics::ForgeDiagnostic;

use crate::discover::discover_packages;

pub fn canonical(text: &str) -> Result<String, ForgeDiagnostic> {
	let mut doc: toml_edit::DocumentMut = text
		.parse()
		.map_err(|e| ForgeDiagnostic::error(101, format!("invalid TOML: {e}")))?;
	format_table(doc.as_table_mut());
	Ok(doc.to_string())
}

fn format_table(table: &mut toml_edit::Table) {
	table.fmt();
	for (_key, item) in table.iter_mut() {
		if let Some(inner) = item.as_table_mut() {
			format_table(inner);
		} else if let Some(inline) = item.as_inline_table_mut() {
			inline.fmt();
		}
		if let Some(array) = item.as_array_mut() {
			array.fmt();
		} else if let Some(value) = item.as_value_mut() {
			if let Some(array) = value.as_array_mut() {
				array.fmt();
			}
			value.decor_mut().clear();
		}
	}
}

fn canonical_file(path: &Path) -> Result<String, ForgeDiagnostic> {
	let original =
		std::fs::read_to_string(path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
	match path.extension().and_then(|e| e.to_str()) {
		Some("toml") => canonical(&original),
		Some("rhai") => crate::rhai_fmt::canonical(&original),
		_ => Ok(original),
	}
}

fn needs_format(path: &Path) -> Result<bool, ForgeDiagnostic> {
	let original =
		std::fs::read_to_string(path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
	let formatted = canonical_file(path)?;
	Ok(formatted != original)
}

fn rewrite_if_needed(path: &Path) -> Result<bool, ForgeDiagnostic> {
	let original =
		std::fs::read_to_string(path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
	let formatted = canonical_file(path)?;
	if formatted == original {
		return Ok(false);
	}
	std::fs::write(path, &formatted).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
	Ok(true)
}

pub fn fmt_workspace(
	workspace: &Path,
	discovery: &crate::workspace::Discovery,
	check_only: bool,
) -> Result<Vec<PathBuf>, ForgeDiagnostic> {
	let packages = discover_packages(workspace, discovery)?;
	let mut files = Vec::new();
	for pkg in &packages {
		files.push(pkg.file.clone());
		if let Some(dir) = pkg.file.parent() {
			for sibling in ["FORGE.toml", "FORGE.rhai"] {
				let candidate = dir.join(sibling);
				if candidate != pkg.file && candidate.is_file() {
					files.push(candidate);
				}
			}
		}
	}
	files.sort();
	files.dedup();
	let mut changed = Vec::new();
	for file in &files {
		let is_formattable = file.extension().is_some_and(|e| e == "toml" || e == "rhai");
		if !is_formattable {
			continue;
		}
		let was_different = if check_only {
			needs_format(file)?
		} else {
			rewrite_if_needed(file)?
		};
		if was_different {
			changed.push(file.clone());
		}
	}
	Ok(changed)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn normalizes_ragged_whitespace_and_is_idempotent() {
		let ugly = "[library.math]\nsrcs = [ \"a.c\" ]\nhdrs   =    [ \"b.h\" ]\n\n[binary.app]\ndeps=[\"math\"]\n\n[binary.app.target.\"os=linux\"]\ndefines     =      [ \"LINUX\" ]\n";
		let once = canonical(ugly).unwrap();
		assert_eq!(
			once,
			"[library.math]\nsrcs = [\"a.c\"]\nhdrs = [\"b.h\"]\n\n[binary.app]\ndeps = [\"math\"]\n\n[binary.app.target.\"os=linux\"]\ndefines = [\"LINUX\"]\n"
		);

		let twice = canonical(&once).unwrap();
		assert_eq!(once, twice);
	}

	#[test]
	fn already_canonical_is_untouched() {
		let clean = "[binary.app]\nsrcs = [\"main.c\"]\ncompiler = \"gcc\"\n";
		assert_eq!(canonical(clean).unwrap(), clean);
	}

	#[test]
	fn workspace_formats_rhai_beside_toml() {
		let dir = std::env::temp_dir().join(format!("forge-fmt-rhai-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(dir.join("c")).unwrap();
		std::fs::create_dir_all(dir.join("r")).unwrap();
		std::fs::create_dir_all(dir.join("b")).unwrap();
		std::fs::write(dir.join("c/FORGE.toml"), "[binary.app]\nsrcs=[\"main.c\"]\n").unwrap();
		std::fs::write(dir.join("r/FORGE.rhai"), "let x=1;\n").unwrap();
		std::fs::write(dir.join("b/FORGE.toml"), "[binary.app]\nsrcs=[\"main.c\"]\n").unwrap();
		std::fs::write(dir.join("b/FORGE.rhai"), "let x=1;\n").unwrap();
		let discovery = crate::workspace::Discovery {
			include: vec![".".into()],
			exclude: vec![],
		};
		let changed = fmt_workspace(&dir, &discovery, false).unwrap();
		assert_eq!(changed.len(), 4);
		assert_eq!(
			std::fs::read_to_string(dir.join("c/FORGE.toml")).unwrap(),
			"[binary.app]\nsrcs = [\"main.c\"]\n"
		);
		assert_eq!(std::fs::read_to_string(dir.join("r/FORGE.rhai")).unwrap(), "let x = 1;\n");
		assert_eq!(
			std::fs::read_to_string(dir.join("b/FORGE.toml")).unwrap(),
			"[binary.app]\nsrcs = [\"main.c\"]\n"
		);
		assert_eq!(std::fs::read_to_string(dir.join("b/FORGE.rhai")).unwrap(), "let x = 1;\n");
		let changed = fmt_workspace(&dir, &discovery, true).unwrap();
		assert!(changed.is_empty());
		std::fs::remove_dir_all(&dir).unwrap();
	}
}
