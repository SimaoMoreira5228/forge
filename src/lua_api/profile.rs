use forge_macros::lua_api;
use mlua::{Lua, Result, Table, UserData, UserDataMethods};
use crate::profile::BuildProfile;

#[derive(Clone)]
pub struct ProfileApi {
	profile: BuildProfile,
}

impl UserData for ProfileApi {
	fn add_methods<M: UserDataMethods<Self>>(_methods: &mut M) {}
}

impl ProfileApi {
	pub fn new(profile: BuildProfile) -> Self {
		Self { profile }
	}
}

#[lua_api(name = "profile")]
impl ProfileApi {
	fn name(&self) -> Result<String> { Ok(self.profile.name.clone()) }
	fn opt_level(&self) -> Result<u8> { Ok(self.profile.opt_level) }
	fn debug(&self) -> Result<bool> { Ok(self.profile.debug) }
	fn lto(&self) -> Result<bool> { Ok(self.profile.lto) }
	fn strip(&self) -> Result<bool> { Ok(self.profile.strip) }
	fn coverage(&self) -> Result<bool> { Ok(self.profile.coverage) }
	fn defines(&self) -> Result<Vec<String>> { Ok(self.profile.defines.clone()) }
	fn sanitizers(&self) -> Result<Vec<String>> { Ok(self.profile.sanitizers.clone()) }
	fn compiler_flags(&self) -> Result<Vec<String>> { Ok(self.profile.compiler_flags.clone()) }
	fn linker_flags(&self) -> Result<Vec<String>> { Ok(self.profile.linker_flags.clone()) }
}

pub fn create_profile_table(lua: &Lua, profile: BuildProfile) -> Result<Table> {
	ProfileApi::new(profile).create_profile_table(lua)
}
