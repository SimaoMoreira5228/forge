use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSpec {
	pub name: String,
	pub command: String,
	pub args: Vec<String>,
	pub inputs: Vec<PathBuf>,
	pub outputs: Vec<PathBuf>,
	pub env: HashMap<String, String>,
	pub workdir: PathBuf,
	pub toolchain_id: Option<String>,
}

impl ActionSpec {
	pub fn new(name: impl Into<String>) -> Self {
		Self {
			name: name.into(),
			command: String::new(),
			args: Vec::new(),
			inputs: Vec::new(),
			outputs: Vec::new(),
			env: HashMap::new(),
			workdir: PathBuf::new(),
			toolchain_id: None,
		}
	}

	pub fn with_command(mut self, command: impl Into<String>) -> Self {
		self.command = command.into();
		self
	}

	pub fn with_args(mut self, args: Vec<String>) -> Self {
		self.args = args;
		self
	}

	pub fn with_inputs(mut self, inputs: Vec<PathBuf>) -> Self {
		self.inputs = inputs;
		self
	}

	pub fn with_outputs(mut self, outputs: Vec<PathBuf>) -> Self {
		self.outputs = outputs;
		self
	}

	pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
		self.env = env;
		self
	}

	pub fn with_workdir(mut self, workdir: PathBuf) -> Self {
		self.workdir = workdir;
		self
	}

	pub fn with_toolchain(mut self, toolchain_id: impl Into<String>) -> Self {
		self.toolchain_id = Some(toolchain_id.into());
		self
	}

	pub fn validate(&self) -> Result<ActionContract, String> {
		let mut missing: Vec<&str> = Vec::new();
		let _unused: Vec<String> = Vec::new();

		if self.command.is_empty() {
			missing.push("command");
		}

		if self.inputs.is_empty() {
			missing.push("inputs");
		}

		if self.outputs.is_empty() {
			missing.push("outputs");
		}

		if !missing.is_empty() {
			return Err(format!("ActionSpec missing required fields: {:?}", missing));
		}

		Ok(ActionContract {
			name: self.name.clone(),
			inputs: self.inputs.clone(),
			outputs: self.outputs.clone(),
			env_keys: self.env.keys().cloned().collect(),
			toolchain_id: self.toolchain_id.clone(),
			verified: false,
		})
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionContract {
	pub name: String,
	pub inputs: Vec<PathBuf>,
	pub outputs: Vec<PathBuf>,
	pub env_keys: Vec<String>,
	pub toolchain_id: Option<String>,
	pub verified: bool,
}

impl ActionContract {
	pub fn verify_execution(&self, actual_inputs: &[PathBuf], actual_outputs: &[PathBuf]) -> Result<(), Vec<String>> {
		let mut violations = Vec::new();

		for input in &self.inputs {
			if !actual_inputs.contains(input) {
				violations.push(format!("Missing declared input: {:?}", input));
			}
		}

		for actual in actual_inputs {
			if !self.inputs.contains(actual) {
				violations.push(format!("Undeclared input accessed: {:?}", actual));
			}
		}

		for output in &self.outputs {
			if !actual_outputs.contains(output) {
				violations.push(format!("Missing declared output: {:?}", output));
			}
		}

		if violations.is_empty() { Ok(()) } else { Err(violations) }
	}
}
