use std::collections::BTreeMap;

use forge_diagnostics::{ForgeDiagnostic, codes};
use serde::Deserialize;

use crate::worker::WorkerProgram;

#[derive(Debug, Default, Clone)]
pub struct Catalog {
	entries: BTreeMap<String, ToolchainEntry>,
}

#[derive(Debug, Clone)]
pub struct ToolchainEntry {
	pub default_version: String,
	pub aliases: Vec<String>,
	pub version_aliases: BTreeMap<String, String>,
	pub targets: BTreeMap<String, TargetUrl>,
	pub install: Option<InstallScript>,
	pub bin_aliases: Vec<BinAlias>,
	pub coverage: Option<CoverageBackend>,
	pub worker: Option<WorkerProgram>,
}

#[derive(Debug, Clone)]
pub struct CoverageBackend {
	pub raw_extension: String,
	pub companions: Vec<String>,
	pub commands: Vec<CoverageCommand>,
	pub format: CoverageFormat,
}

#[derive(Debug, Clone)]
pub struct CoverageCommand {
	pub tool: String,
	pub args: Vec<String>,
	pub object_flag: Option<String>,
	pub per_raw: bool,
	pub stdout: Option<String>,
	pub cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageFormat {
	Lcov,
	Gcov,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinAlias {
	pub from: String,
	pub to: String,
}

#[derive(Debug, Clone)]
pub struct InstallScript {
	pub name: String,
	pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TargetUrl {
	pub url: String,
	pub sha256: Option<String>,
}

impl ToolchainEntry {
	pub fn url_for(&self, version: &str, platform_key: &str) -> Result<TargetUrl, ForgeDiagnostic> {
		let target = self.targets.get(platform_key).ok_or_else(|| {
			ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("toolchain has no download for platform `{platform_key}`"),
			)
		})?;
		Ok(TargetUrl {
			url: target.url.replace("{version}", version),
			sha256: target.sha256.clone(),
		})
	}

	pub fn merge_over(&mut self, override_entry: &ToolchainEntry) {
		if !override_entry.default_version.is_empty() {
			self.default_version = override_entry.default_version.clone();
		}
		for alias in &override_entry.aliases {
			if !self.aliases.contains(alias) {
				self.aliases.push(alias.clone());
			}
		}
		for (version, canonical) in &override_entry.version_aliases {
			self.version_aliases.insert(version.clone(), canonical.clone());
		}
		for (platform, url) in &override_entry.targets {
			self.targets.insert(platform.clone(), url.clone());
		}
		for alias in &override_entry.bin_aliases {
			if !self.bin_aliases.contains(alias) {
				self.bin_aliases.push(alias.clone());
			}
		}
		if override_entry.coverage.is_some() {
			self.coverage = override_entry.coverage.clone();
		}
		if override_entry.install.is_some() {
			self.install = override_entry.install.clone();
		}
		if override_entry.worker.is_some() {
			self.worker = override_entry.worker.clone();
		}
	}
}

#[derive(Deserialize)]
struct RawCoverage {
	raw_extension: String,
	#[serde(default)]
	companions: Vec<String>,
	commands: Vec<RawCoverageCommand>,
	format: String,
}

#[derive(Deserialize)]
struct RawCoverageCommand {
	tool: String,
	#[serde(default)]
	args: Vec<String>,
	#[serde(default)]
	object_flag: Option<String>,
	#[serde(default)]
	per_raw: bool,
	#[serde(default)]
	stdout: Option<String>,
	#[serde(default)]
	cwd: Option<String>,
}

fn parse_coverage(coverage: Option<RawCoverage>) -> Result<Option<CoverageBackend>, ForgeDiagnostic> {
	let Some(coverage) = coverage else {
		return Ok(None);
	};
	let format = match coverage.format.as_str() {
		"lcov" => CoverageFormat::Lcov,
		"gcov" => CoverageFormat::Gcov,
		other => {
			return Err(ForgeDiagnostic::error(101, format!("unknown coverage format `{other}`")));
		}
	};
	Ok(Some(CoverageBackend {
		raw_extension: coverage.raw_extension,
		companions: coverage.companions,
		commands: coverage
			.commands
			.into_iter()
			.map(|command| CoverageCommand {
				tool: command.tool,
				args: command.args,
				object_flag: command.object_flag,
				per_raw: command.per_raw,
				stdout: command.stdout,
				cwd: command.cwd,
			})
			.collect(),
		format,
	}))
}

impl Catalog {
	pub fn parse(toml_text: &str) -> Result<Self, ForgeDiagnostic> {
		#[derive(Deserialize)]
		struct RawCatalog {
			#[serde(default)]
			toolchains: BTreeMap<String, RawEntry>,
		}

		#[derive(Deserialize)]
		struct RawEntry {
			#[serde(default)]
			aliases: Vec<String>,
			default_version: Option<String>,
			#[serde(default)]
			version_aliases: BTreeMap<String, String>,
			#[serde(default)]
			targets: BTreeMap<String, RawTarget>,
			install_script: Option<String>,
			#[serde(default)]
			install_args: Vec<String>,
			#[serde(default)]
			bin_aliases: Vec<RawBinAlias>,
			coverage: Option<RawCoverage>,
			worker: Option<RawWorker>,
		}

		#[derive(Deserialize)]
		struct RawWorker {
			command: String,
			#[serde(default)]
			variants: Vec<String>,
		}

		#[derive(Deserialize)]
		struct RawBinAlias {
			from: String,
			to: String,
		}

		#[derive(Deserialize)]
		struct RawTarget {
			url: String,
			#[serde(default)]
			sha256: Option<String>,
		}

		let raw: RawCatalog =
			toml::from_str(toml_text).map_err(|e| ForgeDiagnostic::error(101, format!("invalid toolchain catalog: {e}")))?;

		let entries = raw
			.toolchains
			.into_iter()
			.map(|(name, e)| {
				Ok((
					name,
					ToolchainEntry {
						default_version: e.default_version.unwrap_or_default(),
						aliases: e.aliases,
						version_aliases: e.version_aliases,
						targets: e
							.targets
							.into_iter()
							.map(|(k, t)| {
								(
									k,
									TargetUrl {
										url: t.url,
										sha256: t.sha256,
									},
								)
							})
							.collect(),
						install: e.install_script.map(|name| InstallScript {
							name,
							args: e.install_args,
						}),
						bin_aliases: e
							.bin_aliases
							.into_iter()
							.map(|alias| BinAlias {
								from: alias.from,
								to: alias.to,
							})
							.collect(),
						coverage: parse_coverage(e.coverage)?,
						worker: e.worker.map(|worker| WorkerProgram {
							command: worker.command,
							variants: worker.variants,
						}),
					},
				))
			})
			.collect::<Result<BTreeMap<_, _>, ForgeDiagnostic>>()?;

		Ok(Self { entries })
	}

	pub fn resolve(
		&self,
		name_or_alias: &str,
		requested_version: Option<&str>,
	) -> Result<ResolvedToolchain, ForgeDiagnostic> {
		let found = self
			.entries
			.iter()
			.find(|(_, e)| e.aliases.iter().any(|a| a == name_or_alias))
			.map(|(n, e)| (n.clone(), e))
			.or_else(|| self.entries.get(name_or_alias).map(|e| (name_or_alias.to_string(), e)))
			.ok_or_else(|| ForgeDiagnostic::error(7, format!("toolchain `{name_or_alias}` is not in the catalog")))?;
		let (name, entry) = found;

		let raw_version = requested_version.unwrap_or(&entry.default_version);
		if raw_version.is_empty() {
			return Err(
				ForgeDiagnostic::error(7, format!("toolchain `{name}` has no version configured"))
					.with_help("set a version in FORGE_ROOT or a default_version in the catalog"),
			);
		}
		let version = entry
			.version_aliases
			.get(raw_version)
			.cloned()
			.unwrap_or_else(|| raw_version.to_string());

		Ok(ResolvedToolchain {
			name: name.clone(),
			version,
			entry: entry.clone(),
		})
	}

	pub fn merge_over(&mut self, overrides: &Catalog) {
		for (name, entry) in &overrides.entries {
			match self.entries.get_mut(name) {
				Some(existing) => existing.merge_over(entry),
				None => {
					self.entries.insert(name.clone(), entry.clone());
				}
			}
		}
	}

	pub fn entry_names(&self) -> impl Iterator<Item = &String> {
		self.entries.keys()
	}

	pub fn get(&self, name: &str) -> Option<&ToolchainEntry> {
		self.entries.get(name)
	}

	pub fn names(&self) -> impl Iterator<Item = &String> {
		self.entries.keys()
	}
}

#[derive(Debug, Clone)]
pub struct ResolvedToolchain {
	pub name: String,
	pub version: String,
	pub entry: ToolchainEntry,
}

#[test]
fn parses_declared_coverage_backend() {
	let cat = Catalog::parse(
		r#"
[toolchains.llvm]
default_version = "1"

[toolchains.llvm.coverage]
raw_extension = "profraw"
format = "lcov"
commands = [
  { tool = "llvm-profdata", args = ["merge", "{raw}"] },
  { tool = "llvm-cov", args = ["export"], stdout = "{work}/coverage.info" },
]
"#,
	)
	.unwrap();
	let backend = cat.get("llvm").unwrap().coverage.as_ref().unwrap();
	assert_eq!(backend.raw_extension, "profraw");
	assert_eq!(backend.format, CoverageFormat::Lcov);
	assert_eq!(backend.commands.len(), 2);
	assert_eq!(backend.commands[1].stdout.as_deref(), Some("{work}/coverage.info"));
}

#[test]
fn worker_declarations_come_from_the_catalog() {
	let cat = Catalog::parse(
		r#"
[toolchains.scala]
default_version = "1"

[toolchains.scala.worker]
command = "scala-worker"
variants = ["batch"]

[toolchains.gcc]
default_version = "14"

[toolchains.gcc.worker]
command = "gcc-batch"
"#,
	)
	.unwrap();
	let scala = cat.get("scala").unwrap().worker.as_ref().unwrap();
	assert_eq!(scala.command, "scala-worker");
	assert!(scala.accepts("batch") && !scala.accepts("incremental"));
	let gcc = cat.get("gcc").unwrap().worker.as_ref().unwrap();
	assert!(gcc.accepts("anything"), "an unlisted variant list accepts every variant");
	assert!(cat.get("rust").is_none());
}

#[test]
fn rejects_unknown_coverage_format() {
	let err = Catalog::parse(
		r#"
[toolchains.x]
default_version = "1"
[toolchains.x.coverage]
raw_extension = "x"
format = "nope"
commands = [{ tool = "t", args = [] }]
"#,
	)
	.unwrap_err();
	assert!(format!("{err}").contains("unknown coverage format `nope`"));
}

#[cfg(test)]
mod tests {
	use super::*;

	const CATALOG: &str = r#"
[toolchains.clang]
aliases = ["llvm"]
default_version = "19.1.7"

[toolchains.clang.targets.linux-x86_64]
url = "https://example.com/llvm-{version}-linux.tar.xz"

[toolchains.gcc]
default_version = "14"

[toolchains.gcc.version_aliases]
"14" = "2025.08"

[toolchains.gcc.targets.linux-x86_64]
url = "https://example.com/gcc-{version}.tar.xz"
"#;

	#[test]
	fn resolves_default_version_and_expands_url() {
		let cat = Catalog::parse(CATALOG).unwrap();
		let r = cat.resolve("clang", None).unwrap();
		assert_eq!(r.version, "19.1.7");
		assert_eq!(
			r.entry.url_for("19.1.7", "linux-x86_64").unwrap().url,
			"https://example.com/llvm-19.1.7-linux.tar.xz"
		);
	}

	#[test]
	fn resolves_by_name_alias() {
		let cat = Catalog::parse(CATALOG).unwrap();
		let r = cat.resolve("llvm", Some("20.1.0")).unwrap();
		assert_eq!(r.name, "clang");
		assert_eq!(r.version, "20.1.0");
	}

	#[test]
	fn expands_version_aliases() {
		let cat = Catalog::parse(CATALOG).unwrap();
		let r = cat.resolve("gcc", None).unwrap();
		assert_eq!(r.version, "2025.08");
		assert!(r.entry.url_for(&r.version, "linux-x86_64").is_ok());
	}

	#[test]
	fn missing_platform_is_actionable() {
		let cat = Catalog::parse(CATALOG).unwrap();
		let r = cat.resolve("gcc", None).unwrap();
		assert!(r.entry.url_for("2025.08", "windows-x86_64").is_err());
	}

	#[test]
	fn unknown_toolchain_errors() {
		let cat = Catalog::parse(CATALOG).unwrap();
		assert!(cat.resolve("dmd", None).is_err());
	}

	#[test]
	fn workspace_catalogs_deep_merge_over_bundled() {
		let mut base = Catalog::parse(CATALOG).unwrap();
		let override_toml = r#"
[toolchains.clang.targets.linux-aarch64]
url = "https://mirror.corp/llvm-{version}-arm64.tar.xz"

[toolchains.mold]
default_version = "2.39.0"

[toolchains.mold.targets.linux-x86_64]
url = "https://github.com/rui314/mold/releases/download/v{version}/mold-{version}-x86_64-linux.tar.gz"

[toolchains.gcc.version_aliases]
"15" = "2026.02"
"#;
		let overrides = Catalog::parse(override_toml).unwrap();
		base.merge_over(&overrides);

		let clang = base.get("clang").unwrap();
		assert!(clang.targets.contains_key("linux-x86_64"), "bundled target must survive");
		assert_eq!(
			clang.targets["linux-aarch64"].url,
			"https://mirror.corp/llvm-{version}-arm64.tar.xz"
		);

		let gcc = base.resolve("gcc", Some("15")).unwrap();
		assert_eq!(gcc.version, "2026.02");
		assert!(base.resolve("gcc", None).is_ok(), "existing aliases survive");

		let mold = base.resolve("mold", None).unwrap();
		assert_eq!(mold.version, "2.39.0");
	}

	#[test]
	fn sha256_is_optional_per_target() {
		let cat = Catalog::parse(
			r#"
[toolchains.t.targets.linux-x86_64]
url = "https://x/y.tar.gz"
sha256 = "aa"
"#,
		)
		.unwrap();
		assert_eq!(cat.get("t").unwrap().targets["linux-x86_64"].sha256.as_deref(), Some("aa"));
	}
}

#[cfg(test)]
mod embedded_catalog_tests {
	use super::*;

	#[test]
	fn bundled_catalog_is_valid() {
		let text = include_str!("../../../../prelude/toolchains/catalog.toml");
		let cat = Catalog::parse(text).expect("bundled catalog must parse");
		for name in cat.names() {
			let resolved = cat
				.resolve(name, None)
				.unwrap_or_else(|e| panic!("toolchain `{name}` does not resolve: {e}"));
			assert!(!resolved.entry.targets.is_empty(), "`{name}` has no download targets");
			assert!(!resolved.version.is_empty());
		}
	}
}
