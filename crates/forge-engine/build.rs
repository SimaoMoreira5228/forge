use std::path::PathBuf;

fn main() {
	let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
	let prelude = manifest_dir.join("../../prelude/std");
	let manifest_path = prelude.join("manifest.toml");
	println!("cargo:rerun-if-changed={}", manifest_path.display());

	let text = std::fs::read_to_string(&manifest_path).expect("read prelude/std/manifest.toml");
	let value: toml::Value = toml::from_str(&text).expect("parse prelude/std/manifest.toml");
	let cells = value
		.get("cells")
		.and_then(toml::Value::as_table)
		.expect("prelude/std/manifest.toml needs a [cells] table");

	let mut generated = String::from("pub const EMBEDDED_CELLS: &[(&str, &str)] = &[\n");
	let mut workspace = String::from("pub const EMBEDDED_WORKSPACE_SCRIPTS: &[(&str, &str)] = &[\n");
	for (name, entry) in cells {
		let script = entry
			.get("script")
			.and_then(toml::Value::as_str)
			.unwrap_or_else(|| panic!("cell `{name}` has no `script` in prelude/std/manifest.toml"));
		let path = prelude.join(script);
		println!("cargo:rerun-if-changed={}", path.display());
		generated.push_str(&format!(
			"\t({name:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../prelude/std/{script}\"))),\n"
		));
		if let Some(workspace_script) = entry.get("workspace_script").and_then(toml::Value::as_str) {
			let path = prelude.join(workspace_script);
			println!("cargo:rerun-if-changed={}", path.display());
			workspace.push_str(&format!(
				"\t({name:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../prelude/std/{workspace_script}\"))),\n"
			));
		}
	}
	generated.push_str("];\n");
	generated.push('\n');
	generated.push_str(&workspace);
	generated.push_str("];\n");

	let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("embedded_cells.rs");
	std::fs::write(out, generated).expect("write embedded_cells.rs");
}
