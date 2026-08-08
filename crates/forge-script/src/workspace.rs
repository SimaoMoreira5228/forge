use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::Profile;
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
	pub catalog_files: Vec<PathBuf>,
	pub max_cache_bytes: Option<u64>,
	pub target_platform: Option<String>,
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
			build: Option<RawBuild>,
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
			#[serde(default = "default_opt")]
			opt_level: u8,
			#[serde(default)]
			debug: bool,
			#[serde(default)]
			lto: bool,
			#[serde(default)]
			strip: bool,
			#[serde(default)]
			coverage: bool,
			#[serde(default)]
			defines: Vec<String>,
			#[serde(default)]
			sanitizers: Vec<String>,
		}

		fn default_opt() -> u8 {
			0
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
		for (name, p) in raw.profile {
			let base = match (&p.inherits, name.as_str()) {
				(Some(parent), _) => profiles.get(parent).cloned().ok_or_else(|| {
					ForgeDiagnostic::error(
						codes::script::WRONG_TYPE,
						format!("profile `{name}` inherits unknown profile `{parent}`"),
					)
				})?,
				(None, "debug") => Profile::debug(),
				(None, _) => Profile::debug(),
			};
			let overlay = Profile {
				name: name.clone(),
				opt_level: p.opt_level,
				debug: p.debug,
				lto: p.lto,
				strip: p.strip,
				defines: p.defines.clone(),
				sanitizers: p.sanitizers.clone(),
				coverage: p.coverage,
			};
			profiles.insert(name, overlay.inherited_from(&base));
		}
		if !profiles.contains_key("debug") {
			profiles.insert("debug".into(), Profile::debug());
		}
		profiles.entry("coverage".into()).or_insert_with(|| Profile {
			name: "coverage".into(),
			coverage: true,
			..Profile::debug()
		});

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
		if let Some(patch) = raw.patch {
			for (cell, entry) in patch.std {
				std_patches.insert(cell, PathBuf::from(entry.path));
			}
		}

		let max_cache_bytes = match raw.build.as_ref().and_then(|b| b.max_cache_size.as_deref()) {
			Some(size) => Some(parse_size(size)?).filter(|n| *n > 0),
			None => None,
		};
		let target_platform = raw.build.and_then(|b| b.target);

		Ok(Self {
			std_patches,
			catalog_files,
			max_cache_bytes,
			target_platform,
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
	use super::parse_size;

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
