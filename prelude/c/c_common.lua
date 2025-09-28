local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = target_common.get_predefined_targets()

M.compilers = {
	gcc = "gcc",
	clang = "clang",
	zig = "zig cc",
}

M.get_host_target = compiler_common.get_host_target
M.get_target_triple_string = compiler_common.get_target_triple_string
M.resolve_sources = compiler_common.resolve_sources
M.resolve_includes = compiler_common.resolve_includes

function M.get_target_directory(target_name, variant_name)
	return target_common.get_target_directory(target_name, variant_name)
end

function M.get_compiler_for_target(compiler, target, compiler_path)
	local args = {}

	if compiler_path then
		return { command = compiler_path, args = {} }
	end

	if compiler == "zig" then
		local zig_target = compiler_common.get_zig_target_string(target)
		return { command = "zig", args = { "cc", "-target", zig_target } }
	elseif compiler == "clang" then
		local clang_target = M.get_target_triple_string(target)
		return { command = "clang", args = { "--target=" .. clang_target } }
	else
		local gcc_cmd = compiler_common.get_gcc_cross_compiler(target, false)
		return { command = gcc_cmd, args = {} }
	end
end

return M
