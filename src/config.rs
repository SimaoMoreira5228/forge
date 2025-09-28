use clap_verbosity_flag::Verbosity;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Config {
	pub verbosity: VerbosityWrapper,
	pub target_filters: Vec<String>,
	pub component_filters: Vec<String>,
	pub test_mode: bool,
}

#[derive(Debug, Clone)]
pub struct VerbosityWrapper(pub Verbosity);

impl Serialize for VerbosityWrapper {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(&self.0.log_level().map_or("off".to_string(), |l| l.to_string()))
	}
}
