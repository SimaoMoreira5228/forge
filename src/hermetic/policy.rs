use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolicyMode {
	Off,
	Warn,
	#[default]
	Strict,
}

impl PolicyMode {
	pub fn is_strict(&self) -> bool {
		matches!(self, PolicyMode::Strict)
	}

	pub fn is_warn(&self) -> bool {
		matches!(self, PolicyMode::Warn)
	}

	pub fn is_off(&self) -> bool {
		matches!(self, PolicyMode::Off)
	}
}

impl std::str::FromStr for PolicyMode {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"off" | "relaxed" => Ok(PolicyMode::Off),
			"warn" => Ok(PolicyMode::Warn),
			"strict" => Ok(PolicyMode::Strict),
			_ => Err(format!("Unknown hermetic mode: {}. Use off, warn, or strict", s)),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermeticPolicy {
	pub mode: PolicyMode,
	pub trace_access: bool,
	pub why_non_hermetic: bool,
}

impl Default for HermeticPolicy {
	fn default() -> Self {
		Self {
			mode: PolicyMode::default(),
			trace_access: false,
			why_non_hermetic: false,
		}
	}
}

impl HermeticPolicy {
	pub fn new(mode: PolicyMode) -> Self {
		Self {
			mode,
			..Default::default()
		}
	}

	pub fn off() -> Self {
		Self::new(PolicyMode::Off)
	}

	pub fn warn() -> Self {
		Self::new(PolicyMode::Warn)
	}

	pub fn strict() -> Self {
		Self::new(PolicyMode::Strict)
	}

	pub fn check_violation(&self, violation: &str) -> Result<(), String> {
		match self.mode {
			PolicyMode::Off => Ok(()),
			PolicyMode::Warn => {
				log::warn!("Hermetic policy violation: {}", violation);
				Ok(())
			}
			PolicyMode::Strict => Err(violation.to_string()),
		}
	}
}
