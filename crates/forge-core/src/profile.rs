use blake3::Hasher;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
	pub name: String,
	pub opt_level: u8,
	pub debug: bool,
	#[serde(default)]
	pub lto: bool,
	#[serde(default)]
	pub strip: bool,
	pub defines: Vec<String>,
	pub sanitizers: Vec<String>,
	#[serde(default)]
	pub coverage: bool,
}

impl Profile {
	pub fn debug() -> Self {
		Self {
			name: "debug".into(),
			opt_level: 0,
			debug: true,
			lto: false,
			strip: false,
			defines: vec!["DEBUG".into()],
			sanitizers: vec![],
			coverage: false,
		}
	}

	pub fn release() -> Self {
		Self {
			name: "release".into(),
			opt_level: 3,
			debug: false,
			lto: true,
			strip: true,
			defines: vec!["NDEBUG".into()],
			sanitizers: vec![],
			coverage: false,
		}
	}

	pub fn inherited_from(&self, base: &Profile) -> Self {
		let mut merged = base.clone();
		merged.name = self.name.clone();
		merged.opt_level = self.opt_level;
		merged.debug = self.debug;
		merged.lto |= self.lto;
		merged.strip |= self.strip;
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
		merged
	}

	pub fn fingerprint(&self) -> String {
		let mut h = Hasher::new();
		h.update(&self.opt_level.to_le_bytes());
		h.update(&[
			u8::from(self.debug),
			u8::from(self.lto),
			u8::from(self.strip),
			u8::from(self.coverage),
		]);
		for d in &self.defines {
			put(&mut h, d);
		}
		for s in &self.sanitizers {
			put(&mut h, s);
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
	fn inheritance_merges_defines_and_sanitizers() {
		let asan = Profile {
			name: "asan".into(),
			sanitizers: vec!["address".into()],
			defines: vec!["ASAN".into()],
			..Profile::debug()
		};
		let merged = asan.inherited_from(&Profile::debug());
		assert_eq!(merged.name, "asan");
		assert!(merged.debug);
		assert_eq!(merged.opt_level, 0);
		assert_eq!(merged.defines, vec!["DEBUG", "ASAN"]);
		assert_eq!(merged.sanitizers, vec!["address"]);
	}

	#[test]
	fn same_settings_same_fingerprint_regardless_of_name() {
		let a = Profile {
			name: "custom".into(),
			..Profile::debug()
		};
		assert_eq!(a.fingerprint(), Profile::debug().fingerprint());
	}
}
