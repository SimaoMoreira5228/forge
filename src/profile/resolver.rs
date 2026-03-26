use super::BuildProfile;
use crate::forge_root_config::ProfileConfig;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProfileError {
	#[error("Profile '{0}' not found (and no builtin defaults exist for this name)")]
	NotFound(String),
	#[error("Circular inheritance detected resolving profile '{0}'")]
	CircularInheritance(String),
}

pub struct ProfileResolver<'a> {
	configs: &'a HashMap<String, ProfileConfig>,
}

impl<'a> ProfileResolver<'a> {
	pub fn new(configs: &'a HashMap<String, ProfileConfig>) -> Self {
		Self { configs }
	}

	pub fn resolve(&self, name: &str) -> Result<BuildProfile, ProfileError> {
		let mut visited = Vec::new();
		let mut chain = Vec::new();
		let mut current_name = name.to_string();

		loop {
			if visited.contains(&current_name) {
				return Err(ProfileError::CircularInheritance(name.to_string()));
			}
			visited.push(current_name.clone());

			let config = self.configs.get(&current_name);
			chain.push((current_name.clone(), config));

			if let Some(config) = config {
				if let Some(parent) = &config.inherits {
					current_name = parent.clone();
					continue;
				}
			}
			break;
		}

		let base_name = chain.last().unwrap().0.as_str();
		let has_base_config = chain.last().unwrap().1.is_some();
		
		let mut final_profile = Self::get_builtin_base(base_name)
			.unwrap_or_else(|| BuildProfile::new(name));

		// If the base name has no config AND is not a builtin, we cannot resolve it
		if !has_base_config && Self::get_builtin_base(base_name).is_none() {
			return Err(ProfileError::NotFound(base_name.to_string()));
		}

		// Ensure the final profile gets the requested top-level name
		final_profile.name = name.to_string();

		for (_, cfg_opt) in chain.into_iter().rev() {
			if let Some(cfg) = cfg_opt {
				Self::apply_config(&mut final_profile, cfg);
			}
		}

		Ok(final_profile)
	}

	fn get_builtin_base(name: &str) -> Option<BuildProfile> {
		match name {
			"debug" => {
				let mut p = BuildProfile::new(name);
				p.opt_level = 0;
				p.debug = true;
				p.defines.push("DEBUG".to_string());
				Some(p)
			}
			"release" => {
				let mut p = BuildProfile::new(name);
				p.opt_level = 3;
				p.debug = false;
				p.lto = true;
				p.strip = true;
				p.defines.push("NDEBUG".to_string());
				Some(p)
			}
			"asan" => {
				let mut p = Self::get_builtin_base("debug").unwrap();
				p.name = name.to_string();
				p.sanitizers.push("address".to_string());
				p.defines.push("ASAN".to_string());
				Some(p)
			}
			"tsan" => {
				let mut p = Self::get_builtin_base("debug").unwrap();
				p.name = name.to_string();
				p.sanitizers.push("thread".to_string());
				p.defines.push("TSAN".to_string());
				Some(p)
			}
			"coverage" => {
				let mut p = Self::get_builtin_base("debug").unwrap();
				p.name = name.to_string();
				p.coverage = true;
				Some(p)
			}
			_ => None,
		}
	}

	fn apply_config(profile: &mut BuildProfile, config: &ProfileConfig) {
		if let Some(val) = config.opt_level { profile.opt_level = val; }
		if let Some(val) = config.debug { profile.debug = val; }
		if let Some(val) = config.lto { profile.lto = val; }
		if let Some(val) = config.strip { profile.strip = val; }
		if let Some(val) = config.coverage { profile.coverage = val; }

		for def in &config.defines {
			if !profile.defines.contains(def) { profile.defines.push(def.clone()); }
		}
		for san in &config.sanitizers {
			if !profile.sanitizers.contains(san) { profile.sanitizers.push(san.clone()); }
		}
		for flag in &config.compiler_flags {
			if !profile.compiler_flags.contains(flag) { profile.compiler_flags.push(flag.clone()); }
		}
		for flag in &config.linker_flags {
			if !profile.linker_flags.contains(flag) { profile.linker_flags.push(flag.clone()); }
		}
	}
}
