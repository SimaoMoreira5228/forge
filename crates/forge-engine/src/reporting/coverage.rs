use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::toolchain::catalog::{CoverageBackend, CoverageCommand, CoverageFormat};
use forge_diagnostics::ForgeDiagnostic;

use crate::toolchain::resolve_tool_path;
use crate::toolchain::store::ResolvedToolchain;

pub struct CoverageReport {
	pub lines_found: usize,
	pub lines_covered: usize,
	pub functions_found: usize,
	pub functions_covered: usize,
	pub lcov_path: Option<PathBuf>,
	pub summary: String,
}

pub fn collect(
	workspace: &Path,
	out_dir: &Path,
	toolchains: &BTreeMap<String, ResolvedToolchain>,
	compiler_hint: Option<&str>,
	profile: &str,
) -> Result<CoverageReport, ForgeDiagnostic> {
	let backend = select_backend(toolchains, compiler_hint)
		.ok_or_else(|| ForgeDiagnostic::error(8, "no configured toolchain declares a coverage backend"))?;
	let work = out_dir.join("coverage");
	std::fs::create_dir_all(&work).map_err(|e| ForgeDiagnostic::error(8, format!("create {}: {e}", work.display())))?;

	let mut raws = Vec::new();
	for file in collect_extension(out_dir, &backend.raw_extension) {
		raws.push(copy_into(&file, &work)?);
	}
	if raws.is_empty() {
		return Err(ForgeDiagnostic::error(
			8,
			"no coverage data found; did the tests run with coverage enabled?",
		));
	}
	for companion in &backend.companions {
		for file in collect_extension(out_dir, companion) {
			copy_into(&file, &work)?;
		}
	}

	let mut report_path = None;
	for command in &backend.commands {
		let tool = cover_tool(toolchains, compiler_hint, &command.tool)?;
		let groups: Vec<Vec<PathBuf>> = if command.per_raw {
			raws.iter().map(|raw| vec![raw.clone()]).collect()
		} else {
			vec![raws.clone()]
		};
		let objects = if command.args.iter().any(|arg| arg == "{objects}") {
			instrumented_objects(out_dir, profile)
		} else {
			Vec::new()
		};
		for group in groups {
			if let Some(path) = run_command(&tool, command, &group, &objects, &work, workspace)? {
				report_path = Some(path);
			}
		}
	}

	match backend.format {
		CoverageFormat::Lcov => {
			let path = report_path
				.ok_or_else(|| ForgeDiagnostic::error(8, "coverage backend declared lcov format but produced no report"))?;
			let text = std::fs::read_to_string(&path)
				.map_err(|e| ForgeDiagnostic::error(8, format!("read {}: {e}", path.display())))?;
			let (lines_found, lines_covered) = parse_lcov(&text, "LF:", "LH:");
			let (functions_found, functions_covered) = parse_lcov(&text, "FNF:", "FNH:");
			Ok(CoverageReport {
				lines_found,
				lines_covered,
				functions_found,
				functions_covered,
				lcov_path: Some(path),
				summary: format!("lines: {lines_covered}/{lines_found}, functions: {functions_covered}/{functions_found}"),
			})
		}
		CoverageFormat::Gcov => {
			let files = collect_extension(workspace, "gcov");
			let (lines_found, lines_covered) = gcov_summary(&files);
			Ok(CoverageReport {
				lines_found,
				lines_covered,
				functions_found: 0,
				functions_covered: 0,
				lcov_path: None,
				summary: format!("lines: {lines_covered}/{lines_found}"),
			})
		}
	}
}

fn select_backend<'a>(
	toolchains: &'a BTreeMap<String, ResolvedToolchain>,
	compiler_hint: Option<&str>,
) -> Option<&'a CoverageBackend> {
	if let Some(hint) = compiler_hint
		&& let Some(backend) = toolchains.get(hint).and_then(|tool| tool.coverage.as_ref())
	{
		return Some(backend);
	}
	toolchains.values().find_map(|tool| tool.coverage.as_ref())
}

fn cover_tool(
	toolchains: &BTreeMap<String, ResolvedToolchain>,
	compiler_hint: Option<&str>,
	name: &str,
) -> Result<PathBuf, ForgeDiagnostic> {
	if let Some(hint) = compiler_hint
		&& let Some(found) = toolchains.get(hint).and_then(|tool| tool.binary(name))
	{
		return Ok(found);
	}
	resolve_tool_path(toolchains, name)
}

fn run_command(
	tool: &Path,
	command: &CoverageCommand,
	raws: &[PathBuf],
	objects: &[PathBuf],
	work: &Path,
	workspace: &Path,
) -> Result<Option<PathBuf>, ForgeDiagnostic> {
	let cwd = command
		.cwd
		.as_deref()
		.map(|dir| substitute(dir, work, workspace))
		.map_or_else(|| workspace.to_path_buf(), PathBuf::from);
	let args: Vec<String> = command
		.args
		.iter()
		.flat_map(|arg| match arg.as_str() {
			"{raw}" => raws.iter().map(|raw| raw.to_string_lossy().into_owned()).collect(),
			"{objects}" => objects.iter().map(|obj| obj.to_string_lossy().into_owned()).collect(),
			_ => vec![substitute(arg, work, workspace)],
		})
		.collect();
	let mut process = std::process::Command::new(tool);
	process.args(&args).current_dir(&cwd);
	let output = process
		.output()
		.map_err(|e| ForgeDiagnostic::error(8, format!("run `{}`: {e}", tool.display())))?;
	if !output.status.success() {
		return Err(coverage_failure(tool, &output.stderr));
	}
	if command.stdout.is_none() {
		return Ok(None);
	}
	let Some(template) = &command.stdout else {
		return Ok(None);
	};
	let path = PathBuf::from(substitute(template, work, workspace));
	std::fs::write(&path, &output.stdout)
		.map_err(|e| ForgeDiagnostic::error(8, format!("write {}: {e}", path.display())))?;
	Ok(Some(path))
}

fn coverage_failure(tool: &Path, stderr: &[u8]) -> ForgeDiagnostic {
	let text = String::from_utf8_lossy(stderr);
	let trimmed = text.trim();
	let detail = if trimmed.is_empty() { "(no stderr)" } else { trimmed };
	ForgeDiagnostic::error(8, format!("coverage tool `{}` failed:\n{detail}", tool.display()))
}

fn substitute(template: &str, work: &Path, workspace: &Path) -> String {
	template
		.replace("{work}", &work.to_string_lossy())
		.replace("{workspace}", &workspace.to_string_lossy())
}

fn copy_into(file: &Path, work: &Path) -> Result<PathBuf, ForgeDiagnostic> {
	let name = file.file_name().unwrap_or_default();
	let destination = work.join(name);
	if destination != file {
		std::fs::copy(file, &destination)
			.map_err(|e| ForgeDiagnostic::error(8, format!("collect {}: {e}", file.display())))?;
	}
	Ok(destination)
}

fn instrumented_objects(out_dir: &Path, profile: &str) -> Vec<PathBuf> {
	let mut objects = Vec::new();
	for root in ["bin", "test"] {
		for file in collect_extension(&out_dir.join(root).join(profile), "") {
			objects.push(file);
		}
	}
	objects
}

fn collect_extension(dir: &Path, extension: &str) -> Vec<PathBuf> {
	let mut files = Vec::new();
	collect_into(dir, extension, &mut files);
	files
}

fn collect_into(dir: &Path, extension: &str, files: &mut Vec<PathBuf>) {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect_into(&path, extension, files);
		} else if extension.is_empty() || path.extension().is_some_and(|e| e == extension) {
			files.push(path);
		}
	}
}

fn gcov_summary(files: &[PathBuf]) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for path in files {
		let Ok(text) = std::fs::read_to_string(path) else {
			continue;
		};
		for line in text.lines() {
			let Some((count, _)) = line.split_once(':') else {
				continue;
			};
			let count = count.trim();
			if count.is_empty() || count == "-" {
				continue;
			}
			found += 1;
			let executed = !count.starts_with("#####") && !count.starts_with("=====");
			if executed && count.trim_end_matches('*').parse::<u64>().is_ok_and(|value| value > 0) {
				covered += 1;
			}
		}
	}
	(found, covered)
}

fn parse_lcov(text: &str, found_tag: &str, covered_tag: &str) -> (usize, usize) {
	let mut found = 0;
	let mut covered = 0;
	for line in text.lines() {
		if let Some(value) = line.strip_prefix(found_tag) {
			found += value.trim().parse().unwrap_or(0);
		} else if let Some(value) = line.strip_prefix(covered_tag) {
			covered += value.trim().parse().unwrap_or(0);
		}
	}
	(found, covered)
}

#[cfg(all(test, unix))]
mod tests {
	use std::os::unix::fs::PermissionsExt;

	use super::*;
	use crate::toolchain::store::ResolvedToolchain;

	fn fake_tool(dir: &Path, name: &str, body: &str) -> PathBuf {
		let path = dir.join(name);
		std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
		path
	}

	#[test]
	fn runs_declared_coverage_backend_and_summarizes_its_report() {
		let base = std::env::temp_dir().join(format!("forge-coverage-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&base);
		let workspace = base.join("ws");
		let out_dir = base.join("out");
		let bin = base.join("bin");
		std::fs::create_dir_all(&workspace).unwrap();
		std::fs::create_dir_all(&out_dir).unwrap();
		std::fs::create_dir_all(&bin).unwrap();

		fake_tool(&bin, "fakecov", "printf 'LF:10\\nLH:7\\nFNF:2\\nFNH:1\\n'");
		std::fs::write(out_dir.join("run.raw"), b"raw").unwrap();

		let backend = forge_core::toolchain::catalog::CoverageBackend {
			raw_extension: "raw".into(),
			companions: vec![],
			commands: vec![forge_core::toolchain::catalog::CoverageCommand {
				tool: "fakecov".into(),
				args: vec![],
				per_raw: false,
				stdout: Some("{work}/coverage.info".into()),
				cwd: None,
			}],
			format: forge_core::toolchain::catalog::CoverageFormat::Lcov,
		};
		let toolchains = BTreeMap::from([(
			"fake".to_string(),
			ResolvedToolchain {
				name: "fake".into(),
				root: bin.clone(),
				bin_dir: bin.clone(),
				path_dirs: vec![bin],
				digest: "deadbeef".into(),
				coverage: Some(backend),
				worker: None,
			},
		)]);

		let report = collect(&workspace, &out_dir, &toolchains, Some("fake"), "coverage").unwrap();
		assert_eq!((report.lines_found, report.lines_covered), (10, 7));
		assert_eq!((report.functions_found, report.functions_covered), (2, 1));
		assert!(report.lcov_path.unwrap().is_file());
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn gcov_summary_counts_missed_lines() {
		let base = std::env::temp_dir().join(format!("forge-gcov-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(&base).unwrap();
		let path = base.join("math.c.gcov");
		std::fs::write(
			&path,
			"        -:    0:Source:math.c\n        3:    1:int add(int a, int b) {\n    #####:    2:    return 0;\n        1:    3:}\n",
		)
		.unwrap();
		assert_eq!(gcov_summary(&[path]), (3, 2));
		let _ = std::fs::remove_dir_all(&base);
	}
}
