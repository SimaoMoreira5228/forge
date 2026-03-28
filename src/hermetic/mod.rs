mod action;
mod policy;
mod runner;
mod sandbox;
mod toolchain;

pub use action::{ActionSpec, ActionInput};
pub use policy::{HermeticPolicy, PolicyMode};
pub use runner::SandboxRunner;
pub use sandbox::{SandboxProvider, get_sandbox};
pub use toolchain::ToolchainFingerprint;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("Policy violation: {0}")]
	PolicyViolation(String),

	#[error("Toolchain error: {0}")]
	ToolchainError(String),

	#[error("Sandbox error: {0}")]
	SandboxError(String),

	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
}

impl From<String> for Error {
	fn from(s: String) -> Self {
		Error::SandboxError(s)
	}
}
