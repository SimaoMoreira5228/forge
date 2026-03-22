local compiler_common = require("@prelude/compiler_common.lua")

local M = {}

local targets_list = forge.target:list()
local predefined = {}
for _, t in ipairs(targets_list) do
	predefined[t.name] = t
end
M.predefined_targets = predefined

M.build_modes = {
	Debug = "Debug",
	ReleaseSafe = "ReleaseSafe",
	ReleaseFast = "ReleaseFast",
	ReleaseSmall = "ReleaseSmall",
}

M.get_host_target = compiler_common.get_host_target
M.get_zig_target_string = compiler_common.get_zig_target_string
M.resolve_includes = compiler_common.resolve_includes
M.resolve_sources = compiler_common.resolve_sources

function M.get_target_directory(target_name, variant_name)
	if variant_name then
		return variant_name
	end

	local resolved = forge.target.resolve(target_name)
	if resolved and resolved.triple then
		return resolved.triple
	end

	return target_name
end

function M.validate_build_mode(mode)
	if not mode then
		return true
	end

	for name, _ in pairs(M.build_modes) do
		if mode == name then
			return true
		end
	end

	return false
end

return M
