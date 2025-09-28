local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = target_common.get_predefined_targets()

M.build_modes = {
	Debug = "Debug",
	ReleaseSafe = "ReleaseSafe",
	ReleaseFast = "ReleaseFast",
	ReleaseSmall = "ReleaseSmall",
}

M.get_host_target = compiler_common.get_host_target
M.get_zig_target_string = compiler_common.get_zig_target_string
M.resolve_includes = compiler_common.resolve_includes

function M.get_target_directory(target_name, variant_name)
	return target_common.get_target_directory(target_name, variant_name)
end

function M.resolve_sources(sources, base_path)
	local resolved = {}
	base_path = base_path or forge.project.root

	for _, src in ipairs(sources) do
		if type(src) == "string" then
			if src:match("%*") then
				local pattern = forge.path.join({ base_path, src })
				local files = forge.fs.glob(pattern)
				for _, file in ipairs(files) do
					table.insert(resolved, file)
				end
			else
				local full_path = forge.path.join({ base_path, src })
				table.insert(resolved, full_path)
			end
		end
	end

	return resolved
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
