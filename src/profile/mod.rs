use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct BuildProfile {
	pub compiler_flags: Vec<String>,
	pub linker_flags: Vec<String>,
}

impl BuildProfile {
	pub fn new() -> Self {
		Self::default()
	}
}
