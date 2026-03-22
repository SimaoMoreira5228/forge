local M = {}
local compiler_common = require("@prelude/compiler_common.lua")

local targets_list = forge.target:list()
local predefined = {}
for _, t in ipairs(targets_list) do
	predefined[t.name] = t
end
M.predefined_targets = predefined

M.compilers = {
	gdc = "gdc",
	dmd = "dmd",
	ldc = "ldc2",
}

function M.get_host_target()
	return forge.target:host()
end

function M.resolve_sources(sources, base_path)
	local result = forge.source.resolve({ patterns = sources })
	return result
end

function M.get_compiler_for_target(compiler, target, compiler_path)
	if compiler_path then
		return { command = compiler_path, args = {} }
	end

	local configured = compiler_common.get_configured_compiler(compiler, false, target)
	if configured then
		return { command = configured, args = {} }
	end

	local cmd = M.compilers[compiler] or "gdc"
	return { command = cmd, args = {} }
end

return M
