use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use std::sync::{Arc, Mutex};
use crate::forge_root_config::ForgeRootConfig;

#[derive(Clone)]
pub struct ProfileApi {
	active_profile: Option<String>,
	config: Arc<Mutex<ForgeRootConfig>>,
}

impl UserData for ProfileApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

impl ProfileApi {
	pub fn new(active_profile: Option<String>, config: Arc<Mutex<ForgeRootConfig>>) -> Self {
		Self { active_profile, config }
	}
}

#[lua_api(name = "profile")]
impl ProfileApi {
	/// Get compiler flags for the active profile
	fn compiler_flags(&self) -> Result<Vec<String>> {
		if let Some(profile_name) = &self.active_profile {
			let config = self.config.lock().unwrap();
			if let Some(profile) = config.profile.get(profile_name) {
				return Ok(profile.compiler_flags.clone());
			}
		}
		Ok(vec![])
	}

	/// Get linker flags for the active profile
	fn linker_flags(&self) -> Result<Vec<String>> {
		if let Some(profile_name) = &self.active_profile {
			let config = self.config.lock().unwrap();
			if let Some(profile) = config.profile.get(profile_name) {
				return Ok(profile.linker_flags.clone());
			}
		}
		Ok(vec![])
	}
}

pub fn create_profile_table(lua: &Lua, active_profile: Option<String>, config: ForgeRootConfig) -> Result<Table> {
	ProfileApi::new(active_profile, Arc::new(Mutex::new(config))).create_profile_table(lua)
}
