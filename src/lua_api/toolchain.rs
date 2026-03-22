use mlua::{Lua, Result, Table, Value};
use forge_macros::lua_api;
use std::path::PathBuf;

#[derive(Clone)]
pub struct ToolchainApi {
    project_root: PathBuf,
}

impl ToolchainApi {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }
}

pub fn create_toolchain_table(lua: &Lua, project_root: PathBuf) -> Result<Table> {
    let api = ToolchainApi::new(project_root);
    api.create_toolchain_table(lua)
}

impl mlua::UserData for ToolchainApi {
    fn add_methods<M: mlua::UserDataMethods<Self>>(_methods: &mut M) {}
}

#[lua_api(name = "toolchain")]
impl ToolchainApi {
    /// Synchronize a toolchain by name and version.
    /// 
    /// This calls the corresponding toolchain driver in `@prelude/toolchains/`.
    pub fn sync(&self, name: String, options: Option<Value>, lua: &Lua) -> Result<Table> {
        let options_tbl = match options {
            Some(Value::Table(t)) => t,
            _ => lua.create_table()?,
        };

        // Load the driver via Lua require
        let driver_name = format!("@prelude/toolchains/{}.lua", name);
        let driver: Table = lua.load(&format!("return require('{}')", driver_name)).eval()?;
        
        let sync_fn: mlua::Function = driver.get("sync")?;
        sync_fn.call((name, options_tbl))
    }

    /// List toolchains configured in FORGE_ROOT.
    pub fn list(&self, lua: &Lua) -> Result<Table> {
        let common: Table = lua.load("return require('@prelude/toolchains/common.lua')").eval()?;
        let list_fn: mlua::Function = common.get("list_configured")?;
        list_fn.call(())
    }

    /// Resolve a toolchain's binaries path without necessarily syncing it.
    pub fn resolve(&self, name: String, options: Option<Value>, lua: &Lua) -> Result<Table> {
        let options_tbl = match options {
            Some(Value::Table(t)) => t,
            _ => lua.create_table()?,
        };

        let common: Table = lua.load("return require('@prelude/toolchains/common.lua')").eval()?;
        let resolve_fn: mlua::Function = common.get("resolve_from_config")?;
        resolve_fn.call((name, options_tbl))
    }
}
