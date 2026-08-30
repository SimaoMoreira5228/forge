use blake3::Hasher;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OptLevel {
	Off,
	Less,
	Default,
	Aggressive,
	Size,
	SizeMin,
}

impl OptLevel {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Off => "0",
			Self::Less => "1",
			Self::Default => "2",
			Self::Aggressive => "3",
			Self::Size => "s",
			Self::SizeMin => "z",
		}
	}
}

impl<'de> Deserialize<'de> for OptLevel {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(untagged)]
		enum Repr {
			Int(i64),
			Text(String),
		}
		let invalid = || D::Error::custom("opt_level must be 0-3, \"s\", or \"z\"");
		match Repr::deserialize(deserializer)? {
			Repr::Int(0) => Ok(Self::Off),
			Repr::Int(1) => Ok(Self::Less),
			Repr::Int(2) => Ok(Self::Default),
			Repr::Int(3) => Ok(Self::Aggressive),
			Repr::Int(_) => Err(invalid()),
			Repr::Text(text) => match text.as_str() {
				"0" => Ok(Self::Off),
				"1" => Ok(Self::Less),
				"2" => Ok(Self::Default),
				"3" => Ok(Self::Aggressive),
				"s" => Ok(Self::Size),
				"z" => Ok(Self::SizeMin),
				_ => Err(invalid()),
			},
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DebugInfo {
	None,
	LineTablesOnly,
	Limited,
	Full,
}

impl DebugInfo {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::None => "none",
			Self::LineTablesOnly => "line-tables-only",
			Self::Limited => "limited",
			Self::Full => "full",
		}
	}
}

impl<'de> Deserialize<'de> for DebugInfo {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(untagged)]
		enum Repr {
			Bool(bool),
			Int(i64),
			Text(String),
		}
		let invalid =
			|| D::Error::custom("debug must be a boolean, 0-2, or \"none\"/\"limited\"/\"full\"/\"line-tables-only\"");
		match Repr::deserialize(deserializer)? {
			Repr::Bool(true) => Ok(Self::Full),
			Repr::Bool(false) => Ok(Self::None),
			Repr::Int(0) => Ok(Self::None),
			Repr::Int(1) => Ok(Self::Limited),
			Repr::Int(2) => Ok(Self::Full),
			Repr::Int(_) => Err(invalid()),
			Repr::Text(text) => match text.as_str() {
				"none" => Ok(Self::None),
				"line-tables-only" => Ok(Self::LineTablesOnly),
				"limited" => Ok(Self::Limited),
				"full" => Ok(Self::Full),
				_ => Err(invalid()),
			},
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lto {
	Off,
	Thin,
	Fat,
}

impl Lto {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Off => "off",
			Self::Thin => "thin",
			Self::Fat => "fat",
		}
	}
}

impl<'de> Deserialize<'de> for Lto {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(untagged)]
		enum Repr {
			Bool(bool),
			Text(String),
		}
		let invalid = || D::Error::custom("lto must be a boolean or \"off\"/\"thin\"/\"fat\"");
		match Repr::deserialize(deserializer)? {
			Repr::Bool(true) => Ok(Self::Fat),
			Repr::Bool(false) => Ok(Self::Off),
			Repr::Text(text) => match text.as_str() {
				"off" | "no" | "n" | "none" => Ok(Self::Off),
				"thin" => Ok(Self::Thin),
				"fat" => Ok(Self::Fat),
				_ => Err(invalid()),
			},
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strip {
	None,
	Debuginfo,
	Symbols,
}

impl Strip {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::None => "none",
			Self::Debuginfo => "debuginfo",
			Self::Symbols => "symbols",
		}
	}
}

impl<'de> Deserialize<'de> for Strip {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(untagged)]
		enum Repr {
			Bool(bool),
			Text(String),
		}
		let invalid = || D::Error::custom("strip must be a boolean or \"none\"/\"debuginfo\"/\"symbols\"");
		match Repr::deserialize(deserializer)? {
			Repr::Bool(true) => Ok(Self::Symbols),
			Repr::Bool(false) => Ok(Self::None),
			Repr::Text(text) => match text.as_str() {
				"none" => Ok(Self::None),
				"debuginfo" => Ok(Self::Debuginfo),
				"symbols" => Ok(Self::Symbols),
				_ => Err(invalid()),
			},
		}
	}
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profile {
	pub name: String,
	pub opt_level: OptLevel,
	pub debug: DebugInfo,
	pub lto: Lto,
	pub strip: Strip,
	pub defines: Vec<String>,
	pub sanitizers: Vec<String>,
	pub coverage: bool,
	pub options: toml::Table,
	pub build: Option<Box<Profile>>,
}

impl Profile {
	pub fn debug() -> Self {
		Self {
			name: "debug".into(),
			opt_level: OptLevel::Off,
			debug: DebugInfo::Full,
			lto: Lto::Off,
			strip: Strip::None,
			defines: vec!["DEBUG".into()],
			sanitizers: vec![],
			coverage: false,
			options: toml::Table::new(),
			build: None,
		}
	}

	pub fn release() -> Self {
		Self {
			name: "release".into(),
			opt_level: OptLevel::Aggressive,
			debug: DebugInfo::None,
			lto: Lto::Off,
			strip: Strip::Symbols,
			defines: vec!["NDEBUG".into()],
			sanitizers: vec![],
			coverage: false,
			options: toml::Table::new(),
			build: None,
		}
	}

	pub fn named(name: impl Into<String>, base: &Profile) -> Self {
		Self {
			name: name.into(),
			..base.clone()
		}
	}

	pub fn inherited_from(&self, base: &Profile) -> Self {
		let mut merged = base.clone();
		merged.name = self.name.clone();
		merged.opt_level = self.opt_level;
		merged.debug = self.debug;
		merged.lto = self.lto;
		merged.strip = self.strip;
		merged.coverage |= self.coverage;
		for d in &self.defines {
			if !merged.defines.contains(d) {
				merged.defines.push(d.clone());
			}
		}
		for s in &self.sanitizers {
			if !merged.sanitizers.contains(s) {
				merged.sanitizers.push(s.clone());
			}
		}
		for (key, value) in &self.options {
			merged.options.insert(key.clone(), value.clone());
		}
		if self.build.is_some() {
			merged.build = self.build.clone();
		}
		merged
	}

	pub fn fingerprint(&self) -> String {
		let mut h = Hasher::new();
		put(&mut h, self.opt_level.as_str());
		put(&mut h, self.debug.as_str());
		put(&mut h, self.lto.as_str());
		put(&mut h, self.strip.as_str());
		h.update(&[u8::from(self.coverage)]);
		for d in &self.defines {
			put(&mut h, d);
		}
		for s in &self.sanitizers {
			put(&mut h, s);
		}
		let mut keys: Vec<&str> = self.options.keys().map(String::as_str).collect();
		keys.sort_unstable();
		for key in keys {
			put(&mut h, key);
			if let Some(value) = self.options.get(key) {
				put(&mut h, &value.to_string());
			}
		}
		if let Some(build) = &self.build {
			put(&mut h, &build.fingerprint());
		}
		h.finalize().to_hex()[..16].to_string()
	}
}

fn put(h: &mut Hasher, s: &str) {
	h.update(&(s.len() as u64).to_le_bytes());
	h.update(s.as_bytes());
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn profiles_differ_in_fingerprint() {
		assert_ne!(Profile::debug().fingerprint(), Profile::release().fingerprint());
	}

	#[test]
	fn inheritance_merges_defines_sanitizers_and_options() {
		let asan = Profile {
			name: "asan".into(),
			sanitizers: vec!["address".into()],
			defines: vec!["ASAN".into()],
			options: toml::Table::from_iter([("codegen-units".into(), toml::Value::Integer(1))]),
			..Profile::debug()
		};
		let merged = asan.inherited_from(&Profile::debug());
		assert_eq!(merged.name, "asan");
		assert_eq!(merged.debug, DebugInfo::Full);
		assert_eq!(merged.opt_level, OptLevel::Off);
		assert_eq!(merged.defines, vec!["DEBUG", "ASAN"]);
		assert_eq!(merged.sanitizers, vec!["address"]);
		assert_eq!(merged.options["codegen-units"].as_integer(), Some(1));
		assert_ne!(merged.fingerprint(), Profile::debug().fingerprint());
	}

	#[test]
	fn same_settings_same_fingerprint_regardless_of_name() {
		let a = Profile::named("custom", &Profile::debug());
		assert_eq!(a.fingerprint(), Profile::debug().fingerprint());
	}

	#[test]
	fn levels_parse_from_ints_strings_and_bools() {
		#[derive(Deserialize)]
		struct Sample {
			opt_level: OptLevel,
			debug: DebugInfo,
			lto: Lto,
			strip: Strip,
		}
		let sample: Sample =
			toml::from_str("opt_level = \"z\"\ndebug = \"line-tables-only\"\nlto = \"thin\"\nstrip = true\n").unwrap();
		assert_eq!(sample.opt_level, OptLevel::SizeMin);
		assert_eq!(sample.debug, DebugInfo::LineTablesOnly);
		assert_eq!(sample.lto, Lto::Thin);
		assert_eq!(sample.strip, Strip::Symbols);
		let sample: Sample = toml::from_str("opt_level = 3\ndebug = 0\nlto = true\nstrip = false\n").unwrap();
		assert_eq!(sample.opt_level, OptLevel::Aggressive);
		assert_eq!(sample.debug, DebugInfo::None);
		assert_eq!(sample.lto, Lto::Fat);
		assert_eq!(sample.strip, Strip::None);
		assert!(toml::from_str::<Sample>("opt_level = 9\ndebug = 0\nlto = \"fat\"\nstrip = false\n").is_err());
	}
}
