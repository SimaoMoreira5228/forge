pub mod visibility {
	pub const VISIBILITY_VIOLATION: u16 = 1;
}

pub mod targets {
	pub const UNKNOWN_TARGET: u16 = 2;
}

pub mod graph {
	pub const CYCLE_DETECTED: u16 = 3;
}

pub mod platform {
	pub const CONSTRAINT_VIOLATION: u16 = 4;
}

pub mod patch {
	pub const PATCH_CONFLICT: u16 = 5;
}

pub mod hermetic {
	pub const HERMETIC_VIOLATION: u16 = 6;
	pub const TOOLCHAIN_MISMATCH: u16 = 7;
}

pub mod inputs {
	pub const MISSING_INPUT: u16 = 8;
}

pub mod script {
	pub const PARSE_ERROR: u16 = 101;
	pub const UNKNOWN_KEY: u16 = 102;
	pub const WRONG_TYPE: u16 = 103;
	pub const BAD_EXPRESSION: u16 = 104;
	pub const DUPLICATE_COMPONENT: u16 = 105;
}
