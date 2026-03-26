pub mod resolver;

#[derive(Debug, Clone, Default)]
pub struct BuildProfile {
	pub name: String,
	pub opt_level: u8,
	pub debug: bool,
	pub lto: bool,
	pub strip: bool,
	pub coverage: bool,
	pub defines: Vec<String>,
	pub sanitizers: Vec<String>,
	pub compiler_flags: Vec<String>,
	pub linker_flags: Vec<String>,
}

impl BuildProfile {
	pub fn new(name: impl Into<String>) -> Self {
		Self {
			name: name.into(),
			..Default::default()
		}
	}

	pub fn fingerprint(&self) -> String {
		let mut hasher = blake3::Hasher::new();
		hasher.update(self.name.as_bytes());
		hasher.update(&[self.opt_level]);
		hasher.update(&[self.debug as u8, self.lto as u8, self.strip as u8, self.coverage as u8]);
		for def in &self.defines { hasher.update(def.as_bytes()); }
		for san in &self.sanitizers { hasher.update(san.as_bytes()); }
		for flag in &self.compiler_flags { hasher.update(flag.as_bytes()); }
		for flag in &self.linker_flags { hasher.update(flag.as_bytes()); }
		hasher.finalize().to_hex().to_string()
	}
}
