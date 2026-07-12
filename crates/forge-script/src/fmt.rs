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

fn needs_format(path: &Path) -> Result<bool, ForgeDiagnostic> {
	let original =
		std::fs::read_to_string(path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
	let formatted = canonical(&original)?;
	Ok(formatted != original)
}

fn rewrite_if_needed(path: &Path) -> Result<bool, ForgeDiagnostic> {
	let original =
		std::fs::read_to_string(path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
	let formatted = canonical(&original)?;
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
	let mut changed = Vec::new();
	for pkg in &packages {
		let is_toml = pkg.file.extension().is_some_and(|e| e == "toml");
		if !is_toml {
			continue;
		}
		let was_different = if check_only {
			needs_format(&pkg.file)?
		} else {
			rewrite_if_needed(&pkg.file)?
		};
		if was_different {
			changed.push(pkg.file.clone());
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
}
