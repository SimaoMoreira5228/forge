use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::{DebugInfo, Lto, OptLevel, Profile, Strip};
use forge_diagnostics::{ForgeDiagnostic, codes};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
	pub name: String,
	pub discovery: Discovery,
	pub toolchains: BTreeMap<String, ToolchainSelection>,
	pub profiles: BTreeMap<String, Profile>,
	pub platforms: BTreeMap<String, forge_core::Platform>,
	pub std_patches: BTreeMap<String, PathBuf>,
	pub cell: BTreeMap<String, toml::Table>,
	pub catalog_files: Vec<PathBuf>,
	pub max_cache_bytes: Option<u64>,
	pub target_platform: Option<String>,
	pub source_mirrors: Vec<(String, String)>,
	pub local_patches: BTreeMap<String, PathBuf>,
	pub git_patches: BTreeMap<String, GitPatch>,
	pub registry_url: Option<String>,
	pub resolution: ResolutionLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionLimits {
	pub max_response_bytes: u64,
	pub timeout_secs: u64,
	pub max_requests: usize,
}

impl Default for ResolutionLimits {
	fn default() -> Self {
		Self {
			max_response_bytes: 8 * 1024 * 1024,
			timeout_secs: 20,
			max_requests: 256,
		}
	}
}

#[derive(Debug, Clone, Default)]
pub struct GitPatch {
	pub git: Option<String>,
	pub rev: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Discovery {
	pub include: Vec<PathBuf>,
	pub exclude: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ToolchainSelection {
	Version {
		version: String,
	},
	Url {
		url: String,
		sha256: Option<String>,
	},
	Path {
		path: PathBuf,
	},
}

impl WorkspaceConfig {
	pub fn parse(text: &str) -> Result<Self, ForgeDiagnostic> {
		#[derive(Deserialize)]
		struct Raw {
			#[serde(default)]
			project: RawProject,
			#[serde(default)]
			discovery: RawDiscovery,
			#[serde(default)]
			toolchains: BTreeMap<String, RawToolchain>,
			#[serde(default)]
			profile: BTreeMap<String, RawProfile>,
			#[serde(default)]
			platforms: BTreeMap<String, RawPlatform>,
			#[serde(default)]
			patch: Option<RawPatch>,
			#[serde(default)]
			catalog: Option<RawCatalogSection>,
			#[serde(default)]
			registry: Option<RawRegistry>,
			#[serde(default)]
			build: Option<RawBuild>,
			#[serde(default)]
			resolution: Option<RawResolution>,
			#[serde(default)]
			deps: Option<toml::Value>,
			#[serde(default)]
			cell: BTreeMap<String, toml::Table>,
		}

		#[derive(Deserialize, Default)]
		#[serde(deny_unknown_fields)]
		struct RawResolution {
			#[serde(default)]
			max_response_bytes: Option<u64>,
			#[serde(default)]
			timeout_secs: Option<u64>,
			#[serde(default)]
			max_requests: Option<usize>,
		}

		#[derive(Deserialize)]
		struct RawBuild {
			#[serde(default)]
			max_cache_size: Option<String>,
			#[serde(default)]
			target: Option<String>,
		}

		#[derive(Deserialize)]
		struct RawCatalogSection {
			#[serde(default)]
			files: Vec<String>,
		}

		#[derive(Deserialize)]
		struct RawPatch {
			#[serde(default)]
			std: BTreeMap<String, RawCellPatch>,
			#[serde(default)]
			local: BTreeMap<String, RawCellPatch>,
			#[serde(default)]
			source: BTreeMap<String, RawSourcePatch>,
			#[serde(default)]
			git: BTreeMap<String, RawGitPatch>,
		}

		#[derive(Deserialize)]
		struct RawSourcePatch {
			mirror: String,
		}

		#[derive(Deserialize)]
		struct RawGitPatch {
			#[serde(default)]
			git: Option<String>,
			#[serde(default)]
			rev: Option<String>,
		}

		#[derive(Deserialize)]
		struct RawRegistry {
			url: String,
		}

		#[derive(Deserialize)]
		struct RawCellPatch {
			path: String,
		}

		#[derive(Deserialize, Default)]
		struct RawProject {
			#[serde(default)]
			name: Option<String>,
		}

		#[derive(Deserialize, Default)]
		struct RawDiscovery {
			#[serde(default)]
			include: Option<Vec<String>>,
			#[serde(default)]
			exclude: Vec<String>,
		}

		#[derive(Deserialize)]
		struct RawToolchain {
			from: String,
			#[serde(default)]
			version: Option<String>,
			#[serde(default)]
			url: Option<String>,
			#[serde(default)]
			sha256: Option<String>,
			#[serde(default)]
			path: Option<String>,
		}

		#[derive(Deserialize)]
		struct RawProfile {
			#[serde(default)]
			inherits: Option<String>,
			#[serde(default)]
			opt_level: Option<OptLevel>,
			#[serde(default)]
			debug: Option<DebugInfo>,
			#[serde(default)]
			lto: Option<Lto>,
			#[serde(default)]
			strip: Option<Strip>,
			#[serde(default)]
			coverage: Option<bool>,
			#[serde(default)]
			defines: Option<Vec<String>>,
			#[serde(default)]
			sanitizers: Option<Vec<String>>,
			#[serde(default)]
			options: Option<toml::Table>,
			#[serde(default)]
			build: Option<Box<RawProfile>>,
		}

		#[derive(Deserialize)]
		struct RawPlatform {
			os: String,
			arch: String,
			#[serde(default)]
			abi: Option<String>,
			#[serde(default)]
			cpu: Option<String>,
		}

		let raw: Raw = toml::from_str(text)
			.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("invalid FORGE_ROOT: {e}")))?;

		let mut toolchains = BTreeMap::new();
		for (name, t) in raw.toolchains {
			let selection = match t.from.as_str() {
				"version" => ToolchainSelection::Version {
					version: t.version.ok_or_else(|| {
						ForgeDiagnostic::error(
							codes::hermetic::TOOLCHAIN_MISMATCH,
							format!("toolchain `{name}` sets from=\"version\" but has no version"),
						)
					})?,
				},
				"path" => ToolchainSelection::Path {
					path: t.path.map(PathBuf::from).ok_or_else(|| {
						ForgeDiagnostic::error(
							codes::hermetic::TOOLCHAIN_MISMATCH,
							format!("toolchain `{name}` sets from=\"path\" but has no path"),
						)
					})?,
				},
				"url" => ToolchainSelection::Url {
					url: t.url.ok_or_else(|| {
						ForgeDiagnostic::error(
							codes::hermetic::TOOLCHAIN_MISMATCH,
							format!("toolchain `{name}` sets from=\"url\" but has no url"),
						)
					})?,
					sha256: t.sha256,
				},
				other => {
					return Err(ForgeDiagnostic::error(
						codes::script::WRONG_TYPE,
						format!("toolchain `{name}` has unknown from=`{other}`"),
					)
					.with_help("expected \"version\", \"url\", or \"path\""));
				}
			};
			toolchains.insert(name, selection);
		}

		let mut profiles: BTreeMap<String, Profile> = BTreeMap::new();
		profiles.insert("debug".into(), Profile::debug());
		profiles.insert("release".into(), Profile::release());
		let overlay = |name: String, p: &RawProfile, base: &Profile| Profile {
			name,
			opt_level: p.opt_level.unwrap_or(base.opt_level),
			debug: p.debug.unwrap_or(base.debug),
			lto: p.lto.unwrap_or(base.lto),
			strip: p.strip.unwrap_or(base.strip),
			coverage: p.coverage.unwrap_or(base.coverage),
			defines: p.defines.clone().unwrap_or_default(),
			sanitizers: p.sanitizers.clone().unwrap_or_default(),
			options: p.options.clone().unwrap_or_default(),
			build: None,
		};
		let mut pending: Vec<(String, RawProfile)> = raw.profile.into_iter().collect();
		while !pending.is_empty() {
			let mut deferred = Vec::new();
			let mut resolved_any = false;
			for (name, p) in pending {
				let base = match &p.inherits {
					Some(parent) => match profiles.get(parent) {
						Some(base) => base.clone(),
						None => {
							deferred.push((name, p));
							continue;
						}
					},
					None if name == "release" => Profile::release(),
					None => Profile::debug(),
				};
				let mut profile = overlay(name.clone(), &p, &base);
				if let Some(build) = &p.build {
					if build.inherits.is_some() {
						return Err(ForgeDiagnostic::error(
							codes::script::WRONG_TYPE,
							format!("profile `{name}` build override cannot inherit"),
						));
					}
					profile.build = Some(Box::new(overlay(format!("{name}.build"), build, &Profile::debug())));
				}
				profiles.insert(name, profile.inherited_from(&base));
				resolved_any = true;
			}
			if !resolved_any {
				let (name, p) = &deferred[0];
				return Err(ForgeDiagnostic::error(
					codes::script::WRONG_TYPE,
					format!(
						"profile `{name}` inherits unknown profile `{}`",
						p.inherits.as_deref().unwrap_or("")
					),
				));
			}
			pending = deferred;
		}
		profiles.entry("coverage".into()).or_insert_with(|| Profile {
			name: "coverage".into(),
			coverage: true,
			..Profile::debug()
		});
		profiles
			.entry("test".into())
			.or_insert_with(|| Profile::named("test", &Profile::debug()));

		let mut platforms = BTreeMap::new();
		for (name, pl) in raw.platforms {
			platforms.insert(
				name,
				forge_core::Platform {
					os: pl.os,
					arch: pl.arch,
					abi: pl.abi,
					cpu: pl.cpu,
				},
			);
		}

		let catalog_files = raw
			.catalog
			.map(|c| c.files.into_iter().map(PathBuf::from).collect())
			.unwrap_or_default();

		let mut std_patches = BTreeMap::new();
		let mut local_patches = BTreeMap::new();
		let mut source_patches = BTreeMap::new();
		let mut git_patches = BTreeMap::new();
		if let Some(patch) = raw.patch {
			for (cell, entry) in patch.std {
				std_patches.insert(cell, PathBuf::from(entry.path));
			}
			for (name, entry) in patch.local {
				local_patches.insert(name, PathBuf::from(entry.path));
			}
			for (prefix, entry) in patch.source {
				source_patches.insert(prefix, entry.mirror);
			}
			for (url, entry) in patch.git {
				git_patches.insert(
					url,
					GitPatch {
						git: entry.git,
						rev: entry.rev,
					},
				);
			}
		}
		let mut source_mirrors: Vec<(String, String)> = source_patches.into_iter().collect();
		source_mirrors.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));

		let max_cache_bytes = match raw.build.as_ref().and_then(|b| b.max_cache_size.as_deref()) {
			Some(size) => Some(parse_size(size)?).filter(|n| *n > 0),
			None => None,
		};
		let target_platform = raw.build.and_then(|b| b.target);

		let registry_url = raw.registry.map(|registry| registry.url);
		if raw.deps.is_some() {
			return Err(ForgeDiagnostic::error(
				codes::script::UNKNOWN_KEY,
				"FORGE_ROOT section [deps] was renamed to [resolution]",
			));
		}
		let defaults = ResolutionLimits::default();
		let resolution = raw.resolution.unwrap_or_default();
		let resolution = ResolutionLimits {
			max_response_bytes: resolution.max_response_bytes.unwrap_or(defaults.max_response_bytes),
			timeout_secs: resolution.timeout_secs.unwrap_or(defaults.timeout_secs),
			max_requests: resolution.max_requests.unwrap_or(defaults.max_requests),
		};
		if resolution.max_response_bytes == 0 || resolution.timeout_secs == 0 || resolution.max_requests == 0 {
			return Err(ForgeDiagnostic::error(
				codes::script::WRONG_TYPE,
				"[resolution] limits must be positive",
			));
		}

		Ok(Self {
			resolution,
			cell: raw.cell,
			std_patches,
			catalog_files,
			max_cache_bytes,
			target_platform,
			source_mirrors,
			local_patches,
			git_patches,
			registry_url,
			name: raw.project.name.unwrap_or_else(|| "unnamed".into()),
			discovery: Discovery {
				include: raw
					.discovery
					.include
					.unwrap_or_else(|| vec![".".into()])
					.into_iter()
					.map(PathBuf::from)
					.collect(),
				exclude: raw.discovery.exclude,
			},
			toolchains,
			profiles,
			platforms,
		})
	}

	pub fn load(root: &Path) -> Result<Self, ForgeDiagnostic> {
		let text = std::fs::read_to_string(root.join("FORGE_ROOT"))
			.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("cannot read FORGE_ROOT: {e}")))?;
		Self::parse(&text)
	}

	pub fn resolve_target(&self) -> Result<forge_core::Platform, ForgeDiagnostic> {
		match &self.target_platform {
			Some(name) => self.platforms.get(name).cloned().ok_or_else(|| {
				ForgeDiagnostic::error(codes::script::UNKNOWN_KEY, format!("unknown target platform `{name}`")).with_help(
					format!(
						"declared platforms: {}",
						self.platforms.keys().cloned().collect::<Vec<_>>().join(", ")
					),
				)
			}),
			None => Ok(forge_core::Platform::host()),
		}
	}

	pub fn resolve_profile(&self, name: &str) -> Result<Profile, ForgeDiagnostic> {
		self.profiles.get(name).cloned().ok_or_else(|| {
			ForgeDiagnostic::error(codes::script::UNKNOWN_KEY, format!("unknown profile `{name}`")).with_help(format!(
				"declared profiles: {}",
				self.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
			))
		})
	}
}

fn parse_size(text: &str) -> Result<u64, ForgeDiagnostic> {
	let trimmed = text.trim();
	let split = trimmed
		.find(|c: char| !c.is_ascii_digit() && c != '.')
		.unwrap_or(trimmed.len());
	let (number, unit) = trimmed.split_at(split);
	let value: f64 = number
		.parse()
		.map_err(|_| ForgeDiagnostic::error(codes::script::WRONG_TYPE, format!("invalid max_cache_size `{text}`")))?;
	let multiplier: f64 = match unit.trim().to_ascii_lowercase().as_str() {
		"" | "b" => 1.0,
		"k" | "kb" => 1e3,
		"ki" | "kib" => 1024.0,
		"m" | "mb" => 1e6,
		"mi" | "mib" => 1024.0 * 1024.0,
		"g" | "gb" => 1e9,
		"gi" | "gib" => 1024.0 * 1024.0 * 1024.0,
		other => {
			return Err(
				ForgeDiagnostic::error(codes::script::WRONG_TYPE, format!("invalid max_cache_size unit `{other}`"))
					.with_help("use b, kb, mb, gb (or kib, mib, gib)"),
			);
		}
	};
	Ok((value * multiplier) as u64)
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::parse_size;

	#[test]
	fn resolution_limits_default_override_and_validate() {
		use super::{ResolutionLimits, WorkspaceConfig};

		assert_eq!(WorkspaceConfig::parse("").unwrap().resolution, ResolutionLimits::default());
		let config =
			WorkspaceConfig::parse("[resolution]\nmax_response_bytes = 32\ntimeout_secs = 1\nmax_requests = 2\n").unwrap();
		assert_eq!(
			config.resolution,
			ResolutionLimits {
				max_response_bytes: 32,
				timeout_secs: 1,
				max_requests: 2
			}
		);
		let partial = WorkspaceConfig::parse("[resolution]\nmax_requests = 17\n").unwrap();
		assert_eq!(partial.resolution.timeout_secs, 20);
		assert_eq!(partial.resolution.max_requests, 17);
		for key in ["max_response_bytes", "timeout_secs", "max_requests"] {
			for value in ["0", "-1", "1.5", "\"unlimited\""] {
				assert!(WorkspaceConfig::parse(&format!("[resolution]\n{key} = {value}\n")).is_err());
			}
		}
		assert!(WorkspaceConfig::parse("[resolution]\nmax_request = 20\n").is_err());
		assert!(
			WorkspaceConfig::parse("[deps]\nmax_requests = 2\n")
				.unwrap_err()
				.to_string()
				.contains("renamed to [resolution]"),
			"the resolution limits section must not keep its old name"
		);
	}

	#[test]
	fn builtin_profiles_and_inheritance_keep_unset_settings() {
		use forge_core::{DebugInfo, Lto, OptLevel, Strip};

		use super::WorkspaceConfig;

		let config = WorkspaceConfig::parse("").unwrap();
		assert_eq!(config.profiles["debug"].opt_level, OptLevel::Off);
		assert_eq!(config.profiles["debug"].debug, DebugInfo::Full);
		let release = &config.profiles["release"];
		assert_eq!(release.opt_level, OptLevel::Aggressive);
		assert_eq!(release.debug, DebugInfo::None);
		assert_eq!(release.lto, Lto::Off);
		assert_eq!(release.strip, Strip::Symbols);
		assert_eq!(release.defines, vec!["NDEBUG"]);
		for builtin in ["debug", "release", "coverage", "test"] {
			assert!(config.profiles.contains_key(builtin), "missing {builtin}");
		}

		let config = WorkspaceConfig::parse("[profile.release]\nopt_level = 2\n").unwrap();
		let release = &config.profiles["release"];
		assert_eq!(release.opt_level, OptLevel::Default);
		assert_eq!(release.debug, DebugInfo::None);
		assert_eq!(release.lto, Lto::Off);
		assert_eq!(release.strip, Strip::Symbols);

		let config = WorkspaceConfig::parse("[profile.asan]\ninherits = \"debug\"\nsanitizers = [\"address\"]\n").unwrap();
		let asan = &config.profiles["asan"];
		assert_eq!(asan.debug, DebugInfo::Full);
		assert_eq!(asan.opt_level, OptLevel::Off);
		assert_eq!(asan.sanitizers, vec!["address"]);

		let config = WorkspaceConfig::parse(
			"[profile.release]\nlto = \"thin\"\nstrip = \"debuginfo\"\n[profile.release.options]\ncodegen-units = 1\npanic = \"abort\"\n[profile.release.build]\nopt_level = 0\ndebug = 1\n",
		)
		.unwrap();
		let release = &config.profiles["release"];
		assert_eq!(release.lto, Lto::Thin);
		assert_eq!(release.strip, Strip::Debuginfo);
		assert_eq!(release.options["codegen-units"].as_integer(), Some(1));
		assert_eq!(release.options["panic"].as_str(), Some("abort"));
		let build = release.build.as_ref().expect("build override");
		assert_eq!(build.opt_level, OptLevel::Off);
		assert_eq!(build.debug, DebugInfo::Limited);
		assert!(
			config
				.profiles
				.values()
				.all(|profile| profile.build.is_none() || profile.name == "release")
		);
	}

	#[test]
	fn invalid_profile_values_are_rejected() {
		use super::WorkspaceConfig;

		for text in [
			"[profile.x]\nopt_level = 9\n",
			"[profile.x]\nopt_level = \"fast\"\n",
			"[profile.x]\ndebug = \"loud\"\n",
			"[profile.x]\nlto = \"maybe\"\n",
			"[profile.x]\nstrip = \"everything\"\n",
			"[profile.asan]\ninherits = \"missing\"\n",
			"[profile.release]\n[profile.release.build]\ninherits = \"release\"\n",
		] {
			assert!(WorkspaceConfig::parse(text).is_err(), "accepted: {text}");
		}
	}

	#[test]
	fn cell_config_preserves_arbitrary_nested_values() {
		let config = crate::workspace::WorkspaceConfig::parse(
			"[cell.demo]\nmode = \"external\"\nenabled = true\n[ cell.demo.options ]\nvalues = [1, 2]\n",
		)
		.unwrap();
		assert_eq!(config.cell["demo"]["mode"].as_str(), Some("external"));
		assert_eq!(config.cell["demo"]["enabled"].as_bool(), Some(true));
		assert_eq!(config.cell["demo"]["options"]["values"].as_array().unwrap().len(), 2);
		assert!(crate::workspace::WorkspaceConfig::parse("").unwrap().cell.is_empty());
	}

	#[test]
	fn source_mirrors_parse_longest_prefix_first() {
		let config = crate::workspace::WorkspaceConfig::parse(
			"[patch.source.\"https://crates.io\"]\nmirror = \"https://mirror.corp\"\n\n[patch.source.\"https://crates.io/api/v1\"]\nmirror = \"https://mirror.corp/api\"\n",
		)
		.unwrap();
		assert_eq!(
			config.source_mirrors,
			vec![
				("https://crates.io/api/v1".to_string(), "https://mirror.corp/api".to_string()),
				("https://crates.io".to_string(), "https://mirror.corp".to_string()),
			]
		);
	}

	#[test]
	fn local_patches_parse_to_workspace_paths() {
		let config = crate::workspace::WorkspaceConfig::parse(
			"[patch.local.dep]\npath = \"vendor/dep\"\n\n[patch.local.other]\npath = \"vendor/other\"\n",
		)
		.unwrap();
		assert_eq!(config.local_patches.get("dep").unwrap(), &PathBuf::from("vendor/dep"));
		assert_eq!(config.local_patches.len(), 2);
	}

	#[test]
	fn git_patches_parse_url_and_revision_overrides() {
		let config = crate::workspace::WorkspaceConfig::parse(
			"[patch.git.\"https://example.com/old\"]\ngit = \"https://example.com/fork\"\nrev = \"abc123\"\n",
		)
		.unwrap();
		let patch = config.git_patches.get("https://example.com/old").unwrap();
		assert_eq!(patch.git.as_deref(), Some("https://example.com/fork"));
		assert_eq!(patch.rev.as_deref(), Some("abc123"));
	}

	#[test]
	fn registry_url_parses() {
		let config = crate::workspace::WorkspaceConfig::parse("[registry]\nurl = \"https://reg.example\"\n").unwrap();
		assert_eq!(config.registry_url.as_deref(), Some("https://reg.example"));
	}

	#[test]
	fn parses_human_cache_sizes() {
		assert_eq!(parse_size("1024").unwrap(), 1024);
		assert_eq!(parse_size("1KB").unwrap(), 1000);
		assert_eq!(parse_size("1KiB").unwrap(), 1024);
		assert_eq!(parse_size("1GB").unwrap(), 1_000_000_000);
		assert_eq!(parse_size("2 GiB").unwrap(), 2 * 1024 * 1024 * 1024);
		assert_eq!(parse_size("500mb").unwrap(), 500_000_000);
		assert!(parse_size("1PB").is_err());
		assert!(parse_size("lots").is_err());
	}
}
