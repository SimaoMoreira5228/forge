use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct Platform {
	pub constraints: HashSet<String>,
}

impl Platform {
	pub fn new() -> Self {
		Self::default()
	}
}
