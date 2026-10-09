use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_core::toolchain::catalog::{CoverageBackend, CoverageCommand, CoverageFormat};
use forge_core::{ActionSpec, OutputDeclaration};
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
	specs: &[ActionSpec],
) -> Result<CoverageReport, ForgeDiagnostic> {
	let compilers: BTreeSet<&str> = specs
		.iter()
		.filter_map(|spec| spec.toolchain_id.as_deref())
		.filter_map(|id| id.split('@').next())
		.filter(|name| toolchains.get(*name).is_some_and(|tool| tool.coverage.is_some()))
		.collect();
	if compiler_hint.is_none() && compilers.len() > 1 {
		return Err(ForgeDiagnostic::error(
			8,
			"selected actions use multiple coverage backends; select targets using one backend",
		));
	}
	let compiler_hint = compiler_hint.or_else(|| compilers.first().copied());
	let backend = select_backend(toolchains, compiler_hint)
		.ok_or_else(|| ForgeDiagnostic::error(8, "no configured toolchain declares a coverage backend"))?;
	let work = out_dir.join("coverage");
	if work.exists() {
		std::fs::remove_dir_all(&work).map_err(|e| ForgeDiagnostic::io(&work, e))?;
	}
	std::fs::create_dir_all(&work).map_err(|e| ForgeDiagnostic::error(8, format!("create {}: {e}", work.display())))?;

	let selected = declared_files(workspace, specs.iter().flat_map(|spec| &spec.outputs));
	let mut raws = Vec::new();
	for file in selected
		.iter()
		.filter(|path| path.extension().is_some_and(|e| e == backend.raw_extension.as_str()))
	{
		let group = work.join(blake3::hash(file.to_string_lossy().as_bytes()).to_hex().as_str());
		raws.push(copy_into(file, &group)?);
	}
	if raws.is_empty() {
		return Err(ForgeDiagnostic::error(
			8,
			"no coverage data found; did the tests run with coverage enabled?",
		));
	}
	for companion in &backend.companions {
		for file in selected
			.iter()
			.filter(|path| path.extension().is_some_and(|e| e == companion.as_str()))
		{
			for raw in &raws {
				if raw.file_stem() == file.file_stem() {
					copy_into(file, raw.parent().expect("collected raw has a parent"))?;
				}
			}
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
			selected
				.iter()
				.filter(|path| {
					path.starts_with(out_dir.join("bin").join("coverage"))
						|| path.starts_with(out_dir.join("test").join("coverage"))
				})
				.cloned()
				.collect()
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
			let files = collect_extension(&work, "gcov");
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
	let raw_name = raws
		.first()
		.and_then(|raw| raw.file_name())
		.unwrap_or_default()
		.to_string_lossy();
	let raw_dir = raws.first().and_then(|raw| raw.parent()).unwrap_or(work).to_string_lossy();
	let expand = |template: &str| {
		substitute(template, work, workspace)
			.replace("{raw_name}", &raw_name)
			.replace("{raw_dir}", &raw_dir)
	};
	let cwd = command
		.cwd
		.as_deref()
		.map(expand)
		.map_or_else(|| workspace.to_path_buf(), PathBuf::from);
	let args: Vec<String> = command
		.args
		.iter()
		.flat_map(|arg| match arg.as_str() {
			"{raw}" => raws.iter().map(|raw| raw.to_string_lossy().into_owned()).collect(),
			"{objects}" => objects
				.iter()
				.flat_map(|obj| {
					command
						.object_flag
						.iter()
						.cloned()
						.chain(std::iter::once(obj.to_string_lossy().into_owned()))
				})
				.collect(),
			_ => vec![expand(arg)],
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
	let path = PathBuf::from(expand(template));
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
	std::fs::create_dir_all(work).map_err(|e| ForgeDiagnostic::io(work, e))?;
	let name = file.file_name().unwrap_or_default();
	let destination = work.join(name);
	if destination != file {
		if destination.exists() {
			return Err(ForgeDiagnostic::error(
				8,
				format!("coverage artifacts share the filename `{}`", name.to_string_lossy()),
			));
		}
		std::fs::copy(file, &destination)
			.map_err(|e| ForgeDiagnostic::error(8, format!("collect {}: {e}", file.display())))?;
	}
	Ok(destination)
}

fn declared_files<'a>(workspace: &Path, outputs: impl Iterator<Item = &'a OutputDeclaration>) -> BTreeSet<PathBuf> {
	let mut files = BTreeSet::new();
	for output in outputs {
		let path = workspace.join(&output.path);
		if path.is_dir() {
			files.extend(collect_extension(&path, ""));
		} else if path.is_file() {
			files.insert(path);
		}
	}
	files
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
	let mut lines = BTreeMap::<(String, u64), bool>::new();
	for path in files {
		let Ok(text) = std::fs::read_to_string(path) else {
			continue;
		};
		let mut source = path.to_string_lossy().into_owned();
		for line in text.lines() {
			let mut fields = line.splitn(3, ':');
			let count = fields.next().unwrap_or_default().trim();
			let number = fields.next().and_then(|n| n.trim().parse::<u64>().ok());
			let contents = fields.next().unwrap_or_default();
			if let Some(name) = contents.strip_prefix("Source:") {
				source = name.to_string();
				continue;
			}
			let Some(number) = number.filter(|n| *n > 0) else {
				continue;
			};
			if count.is_empty() || count == "-" {
				continue;
			}
			let executed = count.trim_end_matches('*').parse::<u64>().is_ok_and(|value| value > 0);
			*lines.entry((source.clone(), number)).or_default() |= executed;
		}
	}
	(lines.len(), lines.values().filter(|executed| **executed).count())
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
				object_flag: None,
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

		let outputs = vec![OutputDeclaration {
			path: out_dir.join("run.raw"),
			kind: forge_core::OutputKind::File,
		}];
		let files = declared_files(&workspace, outputs.iter());
		assert_eq!(files, BTreeSet::from([out_dir.join("run.raw")]));
		let specs = vec![ActionSpec {
			name: "coverage".into(),
			component: "//:coverage".into(),
			configuration: forge_core::ConfigTransition::Target,
			command: String::new(),
			args: vec![],
			inputs: vec![],
			execution_deps: vec![],
			outputs,
			workdir: None,
			is_test: true,
			stdout: None,
			compile_command: None,
			environment_files: vec![],
			argument_files: vec![],
			env: BTreeMap::new(),
			toolchain_ids: vec![],
			toolchain_id: None,
			worker: None,
		}];
		std::fs::write(out_dir.join("unselected.raw"), b"unselected").unwrap();
		std::fs::create_dir_all(out_dir.join("coverage")).unwrap();
		std::fs::write(out_dir.join("coverage/stale.raw"), b"stale").unwrap();
		let report = collect(&workspace, &out_dir, &toolchains, Some("fake"), &specs).unwrap();
		assert!(!out_dir.join("coverage/unselected.raw").exists());
		assert!(!out_dir.join("coverage/stale.raw").exists());
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
