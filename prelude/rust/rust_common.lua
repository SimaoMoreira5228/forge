local toolchains = require("@prelude/toolchains/init.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

function M.resolve_cargo(target)
	local platform_config = forge.config.platforms[target]
	local toolchain_name = (platform_config and platform_config.toolchain) or "rust"
	local info = toolchains.resolve_compiler(toolchain_name)
	if not info or not info.cargo then
		-- Fallback to system cargo if not found in toolchain
		if forge.fs.exists("/usr/bin/cargo") then return "/usr/bin/cargo" end
		if forge.fs.exists("/usr/local/bin/cargo") then return "/usr/local/bin/cargo" end
		return "cargo"
	end
	return info.cargo
end

function M.get_target_dir(name, target, profile)
	return "target/" .. target .. "/" .. (profile or "debug") .. "/" .. name
end

function M.get_cargo_env(target, target_dir)
	local platform_config = forge.config.platforms[target]
	local toolchain_name = (platform_config and platform_config.toolchain) or "rust"
	local info = toolchains.resolve_compiler(toolchain_name)
	
	local env = {
		CARGO_TARGET_DIR = target_dir,
		CARGO_HOME = forge.path.join({ forge.project.root, ".forge", "cargo" }),
	}

	if info and info.paths then
		env.PATH = table.concat(info.paths, ":") .. ":/usr/bin:/bin"
	else
		env.PATH = "/usr/bin:/bin"
	end

	return env
end

return M
