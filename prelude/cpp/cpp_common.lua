local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = target_common.get_predefined_targets()

M.compilers = {
	gcc = "g++",
	clang = "clang++",
	zig = "zig c++",
}

M.standards = {
	cpp98 = "c++98",
	cpp03 = "c++03",
	cpp11 = "c++11",
	cpp14 = "c++14",
	cpp17 = "c++17",
	cpp20 = "c++20",
	cpp23 = "c++23",
	cpp26 = "c++26",
}

M.get_host_target = compiler_common.get_host_target
M.get_target_triple_string = compiler_common.get_target_triple_string
M.resolve_sources = compiler_common.resolve_sources
M.resolve_includes = compiler_common.resolve_includes

function M.get_compiler_for_target(compiler, target, standard, compiler_path)
	local std_flag = standard and ("-std=" .. standard) or nil
	local args = {}

	if compiler_path then
		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = compiler_path, args = args }
	end

	if compiler == "zig" then
		local zig_target = compiler_common.get_zig_target_string(target)
		table.insert(args, "c++")
		table.insert(args, "-target")
		table.insert(args, zig_target)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = "zig", args = args }
	elseif compiler == "clang" then
		local clang_target = M.get_target_triple_string(target)
		table.insert(args, "--target=" .. clang_target)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = "clang++", args = args }
	else
		local cpp_cmd = compiler_common.get_gcc_cross_compiler(target, true)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = cpp_cmd, args = args }
	end
end

function M.validate_standard(standard)
	if not standard then
		return nil
	end

	if M.standards[standard] then
		return M.standards[standard]
	end

	if forge.string.starts_with(standard, "c++") then
		return standard
	end

	forge.log.warn(("Unknown C++ standard: %s, using default"):format(standard))
	return nil
end

return M
