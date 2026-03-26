use crate::error::ForgeError;
use crate::project::Project;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;

pub fn aggregate_coverage(project: &Project, output_format: Option<&str>) -> Result<(), ForgeError> {
	log::info!("Aggregating test coverage...");

	let coverage_dir = project.path.join("forge-out").join("coverage");
	if !coverage_dir.exists() {
		log::warn!("No coverage data found in {}", coverage_dir.display());
		return Ok(());
	}

	// 1. Locate toolchain binaries
	let toolchain_paths = project.get_toolchain_paths();
	let llvm_profdata = find_tool("llvm-profdata", &toolchain_paths)
		.ok_or_else(|| ForgeError::Other(anyhow::anyhow!("llvm-profdata not found in toolchain paths")))?;
	let llvm_cov = find_tool("llvm-cov", &toolchain_paths)
		.ok_or_else(|| ForgeError::Other(anyhow::anyhow!("llvm-cov not found in toolchain paths")))?;

	log::debug!("Using llvm-profdata: {}", llvm_profdata.display());
	log::debug!("Using llvm-cov: {}", llvm_cov.display());

	// 2. Find all .profraw files
	let mut profraw_files = Vec::new();
	if let Ok(entries) = fs::read_dir(&coverage_dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("profraw") {
				profraw_files.push(path);
			}
		}
	}

	if profraw_files.is_empty() {
		log::warn!("No .profraw files found in {}", coverage_dir.display());
		return Ok(());
	}

	// 3. Merge profiles
	let merged_profraw = coverage_dir.join("merged.profdata");
	let mut merge_cmd = Command::new(&llvm_profdata);
	merge_cmd.arg("merge").arg("-sparse");
	for file in profraw_files {
		merge_cmd.arg(file);
	}
	merge_cmd.arg("-o").arg(&merged_profraw);

	log::info!("Merging {} profile(s) into {}", merge_cmd.get_args().count() - 3, merged_profraw.display());
	let output = merge_cmd.output().map_err(|e| ForgeError::Other(anyhow::anyhow!("Failed to run llvm-profdata: {}", e)))?;
	if !output.status.success() {
		return Err(ForgeError::Other(anyhow::anyhow!(
			"llvm-profdata failed: {}",
			String::from_utf8_lossy(&output.stderr)
		)));
	}

	// 4. Find test binaries
	let mut test_binaries = Vec::new();
	{
		let dep_graph = project.dependency_graph.lock().unwrap();
		for id in dep_graph.component_ids() {
			if let Some(comp) = dep_graph.get_component(id) {
				if let crate::graph::ComponentType::Test { executable: Some(exec), .. } = &comp.component_type {
					let abs_exec = if exec.is_absolute() {
						exec.clone()
					} else {
						project.path.join(exec)
					};
					// We need to check for both the original and the _t_bin version
					if abs_exec.exists() {
						test_binaries.push(abs_exec);
					} else {
						let t_bin = abs_exec.with_file_name(format!("{}_t_bin", abs_exec.file_name().unwrap_or_default().to_string_lossy()));
						if t_bin.exists() {
							test_binaries.push(t_bin);
						}
					}
				}
			}
		}
	}

	if test_binaries.is_empty() {
		log::warn!("No test binaries found to generate coverage report from.");
		return Ok(());
	}

	// 5. Generate report
	let (format_type, output_path) = if let Some(out) = output_format {
		if let Some(idx) = out.find(':') {
			(&out[..idx], Some(&out[idx + 1..]))
		} else {
			(out, None)
		}
	} else {
		("text", None)
	};

	let mut cov_cmd = Command::new(&llvm_cov);
	match format_type {
		"html" => {
			cov_cmd.arg("show").arg("-format=html");
			if let Some(path) = output_path {
				cov_cmd.arg(format!("-output-dir={}", path));
			}
		}
		"lcov" => {
			cov_cmd.arg("export").arg("-format=lcov");
		}
		"json" => {
			cov_cmd.arg("export").arg("-format=text"); // llvm-cov export defaults to JSON
		}
		"text" | "summary" => {
			cov_cmd.arg("report");
		}
		_ => {
			log::warn!("Unknown coverage output format: {}. Defaulting to summary.", format_type);
			cov_cmd.arg("report");
		}
	}

	cov_cmd.arg("-instr-profile").arg(&merged_profraw);
	
	for (i, bin) in test_binaries.iter().enumerate() {
		if i == 0 {
			cov_cmd.arg(bin);
		} else {
			cov_cmd.arg("-object").arg(bin);
		}
	}

	log::info!("Generating {} coverage report...", format_type);
	let output = cov_cmd.output().map_err(|e| ForgeError::Other(anyhow::anyhow!("Failed to run llvm-cov: {}", e)))?;
	
	if !output.status.success() {
		return Err(ForgeError::Other(anyhow::anyhow!(
			"llvm-cov failed: {}",
			String::from_utf8_lossy(&output.stderr)
		)));
	}

	if format_type == "text" || format_type == "summary" {
		println!("\n=== Coverage Summary ===");
		println!("{}", String::from_utf8_lossy(&output.stdout));
	} else if let Some(path) = output_path {
		if format_type != "html" {
			fs::write(path, &output.stdout).map_err(|e| ForgeError::Other(anyhow::anyhow!("Failed to write coverage report to {}: {}", path, e)))?;
			log::info!("Coverage report written to {}", path);
		} else {
			log::info!("HTML coverage report generated in {}", path);
		}
	} else {
		// Output to stdout if no path provided
		println!("{}", String::from_utf8_lossy(&output.stdout));
	}

	Ok(())
}

fn find_tool(name: &str, paths: &[PathBuf]) -> Option<PathBuf> {
	for path in paths {
		let tool = path.join(name);
		#[cfg(windows)]
		let tool = tool.with_extension("exe");
		if tool.exists() {
			return Some(tool);
		}
	}
	None
}
