use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_diagnostics::ForgeDiagnostic;

use crate::toolchain::store::ResolvedToolchain;

pub struct CoverageReport {
	pub lines_found: usize,
	pub lines_covered: usize,
	pub functions_found: usize,
	pub functions_covered: usize,
	pub summary: String,
}

pub enum CoverageFormat {
	Lcov,
	Text,
}

pub fn merge_profiles(
	workspace: &Path,
	out_dir: &Path,
	toolchains: &BTreeMap<String, ResolvedToolchain>,
	compiler_hint: Option<&str>,
) -> Result<Vec<PathBuf>, ForgeDiagnostic> {
	let mut raw_files = Vec::new();
	collect_profraw(out_dir, &mut raw_files);
	if !raw_files.is_empty() && find_tool(toolchains, "llvm-profdata", compiler_hint)?.is_some() {
		let merged = out_dir.join("merged.profdata");
		let tool = find_tool(toolchains, "llvm-profdata", compiler_hint)?
			.ok_or_else(|| ForgeDiagnostic::error(8, "llvm-profdata not found in any toolchain"))?;
		let mut args: Vec<&str> = vec!["merge"];
		args.extend(raw_files.iter().filter_map(|f| f.to_str()));
		args.push("-o");
		args.push(merged.to_str().unwrap());
		let status = std::process::Command::new(&tool)
			.args(&args)
			.status()
			.map_err(|e| ForgeDiagnostic::error(8, format!("failed to run llvm-profdata: {e}")))?;
		if !status.success() {
			return Err(ForgeDiagnostic::error(8, "llvm-profdata merge failed"));
		}
		return Ok(vec![merged]);
	}
	let gcov_files = find_gcda_files(out_dir);
	if gcov_files.is_empty() {
		return Ok(Vec::new());
	}
	let tool = find_tool(toolchains, "gcov", compiler_hint)?
		.ok_or_else(|| ForgeDiagnostic::error(8, "gcov not found in any toolchain"))?;
	let object_dir = out_dir.join("profile/coverage");
	std::fs::create_dir_all(&object_dir)
		.map_err(|e| ForgeDiagnostic::error(8, format!("failed to create {}: {e}", object_dir.display())))?;
	for gcno in find_gcno_files(out_dir) {
		let local_gcno = object_dir.join(gcno.file_name().unwrap_or_default());
		if local_gcno != gcno {
			std::fs::copy(&gcno, &local_gcno)
				.map_err(|e| ForgeDiagnostic::error(8, format!("failed to collect {}: {e}", gcno.display())))?;
		}
	}
	for gcda in &gcov_files {
		let local_gcda = object_dir.join(gcda.file_name().unwrap_or_default());
		if local_gcda != *gcda {
			std::fs::copy(gcda, &local_gcda)
				.map_err(|e| ForgeDiagnostic::error(8, format!("failed to collect {}: {e}", gcda.display())))?;
		}
		let _ = std::process::Command::new(&tool)
			.args(["-o", object_dir.to_str().unwrap_or_default()])
			.arg(&local_gcda)
			.current_dir(workspace)
			.status();
	}
	Ok(find_gcov_files(workspace))
}

pub fn generate_report(
	workspace: &Path,
	out_dir: &Path,
	profiles: &[PathBuf],
	toolchains: &BTreeMap<String, ResolvedToolchain>,
	format: CoverageFormat,
	compiler_hint: Option<&str>,
) -> Result<CoverageReport, ForgeDiagnostic> {
	if let Some(profdata) = profiles.iter().find(|p| p.extension().is_some_and(|e| e == "profdata")) {
		let tool = find_tool(toolchains, "llvm-cov", compiler_hint)?
			.ok_or_else(|| ForgeDiagnostic::error(8, "llvm-cov not found in any toolchain"))?;
		match format {
			CoverageFormat::Text => {
				let output = std::process::Command::new(&tool)
					.args(["report", "-instr-profile"])
					.arg(profdata)
					.current_dir(workspace)
					.output()
					.map_err(|e| ForgeDiagnostic::error(8, format!("llvm-cov report failed: {e}")))?;
				let stdout = String::from_utf8_lossy(&output.stdout).to_string();
				let stderr = String::from_utf8_lossy(&output.stderr).to_string();
				let (lines_found, lines_covered) = parse_text_summary(&stdout);
				let (functions_found, functions_covered) = parse_text_functions(&stdout);
				Ok(CoverageReport {
					lines_found,
					lines_covered,
					functions_found,
					functions_covered,
					summary: format!("{}\n{}", stdout.trim(), stderr.trim()),
				})
			}
			CoverageFormat::Lcov => {
				let output_path = out_dir.join("coverage.info");
				let output = std::process::Command::new(&tool)
					.args(["export", "-instr-profile"])
					.arg(profdata)
					.args(["-format", "lcov"])
					.current_dir(workspace)
					.output()
					.map_err(|e| ForgeDiagnostic::error(8, format!("llvm-cov export failed: {e}")))?;
				let stdout = String::from_utf8_lossy(&output.stdout).to_string();
				let _stderr = String::from_utf8_lossy(&output.stderr).to_string();
				std::fs::write(&output_path, &stdout)
					.map_err(|e| ForgeDiagnostic::error(8, format!("failed to write {}: {e}", output_path.display())))?;
				let (lines_found, lines_covered) = parse_lcov_summary(&stdout);
				let (functions_found, functions_covered) = parse_lcov_functions(&stdout);
				Ok(CoverageReport {
					lines_found,
					lines_covered,
					functions_found,
					functions_covered,
					summary: format!("wrote {}", output_path.display()),
				})
			}
		}
	} else {
		let gcov_files: Vec<&PathBuf> = profiles
			.iter()
			.filter(|p| p.extension().is_some_and(|e| e == "gcov"))
			.collect();
		if !gcov_files.is_empty() {
			let (lines_found, lines_covered) = gcov_summary(&gcov_files);
			return Ok(CoverageReport {
				lines_found,
				lines_covered,
				functions_found: 0,
				functions_covered: 0,
				summary: format!("lines: {lines_covered}/{lines_found}"),
			});
		}
		let lcov_path = out_dir.join("coverage.info");
		if lcov_path.exists() {
			let lcov = std::fs::read_to_string(&lcov_path)
				.map_err(|e| ForgeDiagnostic::error(8, format!("failed to read {}: {e}", lcov_path.display())))?;
			let (lines_found, lines_covered) = parse_lcov_summary(&lcov);
			let (functions_found, functions_covered) = parse_lcov_functions(&lcov);
			Ok(CoverageReport {
				lines_found,
				lines_covered,
				functions_found,
				functions_covered,
				summary: format!("found {}", lcov_path.display()),
			})
		} else {
			Err(ForgeDiagnostic::error(
				8,
				"no coverage data found; did the tests run with coverage enabled?",
			))
		}
	}
}

fn gcov_summary(files: &[&PathBuf]) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for path in files {
		let Ok(text) = std::fs::read_to_string(path) else { continue };
		for line in text.lines() {
			let Some((count, _)) = line.split_once(':') else { continue };
			let count = count.trim();
			if count == "-" || count.is_empty() {
				continue;
			}
			if let Ok(value) = count.parse::<u64>() {
				found += 1;
				covered += usize::from(value > 0);
			}
		}
	}
	(found, covered)
}

fn find_tool(
	toolchains: &BTreeMap<String, ResolvedToolchain>,
	name: &str,
	compiler_hint: Option<&str>,
) -> Result<Option<PathBuf>, ForgeDiagnostic> {
	if let Some(hint) = compiler_hint
		&& let Some(tool) = toolchains.get(hint)
	{
		return Ok(tool.binary(name));
	}
	for tool in toolchains.values() {
		if let Some(found) = tool.binary(name) {
			return Ok(Some(found));
		}
	}
	Ok(None)
}

fn collect_profraw(dir: &Path, files: &mut Vec<PathBuf>) {
	if let Ok(entries) = std::fs::read_dir(dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				collect_profraw(&path, files);
			} else if path.extension().is_some_and(|e| e == "profraw") {
				files.push(path);
			}
		}
	}
}

fn find_gcda_files(dir: &Path) -> Vec<PathBuf> {
	let mut files = Vec::new();
	collect_extension(dir, "gcda", &mut files);
	files
}

fn find_gcov_files(dir: &Path) -> Vec<PathBuf> {
	let mut files = Vec::new();
	collect_extension(dir, "gcov", &mut files);
	files
}

fn find_gcno_files(dir: &Path) -> Vec<PathBuf> {
	let mut files = Vec::new();
	collect_extension(dir, "gcno", &mut files);
	files
}

fn collect_extension(dir: &Path, extension: &str, files: &mut Vec<PathBuf>) {
	let Ok(entries) = std::fs::read_dir(dir) else { return };
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect_extension(&path, extension, files);
		} else if path.extension().is_some_and(|e| e == extension) {
			files.push(path);
		}
	}
}

fn parse_text_summary(text: &str) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for line in text.lines() {
		if line.contains("Lines covered:")
			&& let Some(rest) = line.split_once("Lines covered:").map(|(_, r)| r.trim())
		{
			let parts: Vec<&str> = rest.split('/').collect();
			if parts.len() == 2 {
				covered = parts[0].trim().parse().unwrap_or(0);
				found = parts[1].trim().parse().unwrap_or(0);
			}
		}
	}
	(found, covered)
}

fn parse_text_functions(text: &str) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for line in text.lines() {
		if (line.contains("Functions covered:") || line.contains("Functions executed:"))
			&& let Some(rest) = line
				.split_once("Functions covered:")
				.or_else(|| line.split_once("Functions executed:"))
				.map(|(_, r)| r.trim())
		{
			let parts: Vec<&str> = rest.split('/').collect();
			if parts.len() == 2 {
				covered = parts[0].trim().parse().unwrap_or(0);
				found = parts[1].trim().parse().unwrap_or(0);
			}
		}
	}
	(found, covered)
}

fn parse_lcov_summary(text: &str) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for line in text.lines() {
		if let Some(value) = line.strip_prefix("LF:") {
			found += value.trim().parse().unwrap_or(0);
		} else if let Some(value) = line.strip_prefix("LH:") {
			covered += value.trim().parse().unwrap_or(0);
		}
	}
	(found, covered)
}

fn parse_lcov_functions(text: &str) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for line in text.lines() {
		if let Some(value) = line.strip_prefix("FNF:") {
			found += value.trim().parse().unwrap_or(0);
		} else if let Some(value) = line.strip_prefix("FNH:") {
			covered += value.trim().parse().unwrap_or(0);
		}
	}
	(found, covered)
}
