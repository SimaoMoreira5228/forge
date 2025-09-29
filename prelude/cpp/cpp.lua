local common = require("@prelude/cpp/cpp_common.lua")
local compiler = require("@prelude/cpp/cpp_compiler.lua")

local M = {}

M.predefined_targets = common.predefined_targets
M.compilers = common.compilers
M.standards = common.standards

local defined_libraries = {}

local function validate_library(tbl)
	if not tbl.name then
		error("Library definition must include a 'name' field")
	end

	if not tbl.targets then
		error(("Library '%s' must specify targets"):format(tbl.name))
	end

	for target_name, target_config in pairs(tbl.targets) do
		if not target_config.target then
			error(("Library '%s' target '%s' must specify a target"):format(tbl.name, target_name))
		end

		local compiler_name = target_config.compiler or "gcc"
		if not M.compilers[compiler_name] then
			forge.log.warn(("Unknown compiler '%s' for library '%s' target '%s'"):format(compiler_name, tbl.name, target_name))
		end

		if target_config.standard then
			local valid_std = common.validate_standard(target_config.standard)
			if not valid_std then
				forge.log.warn(
					("Invalid C++ standard '%s' for library '%s' target '%s'"):format(target_config.standard, tbl.name, target_name)
				)
			end
		end
	end

	return true
end

local function validate_binary(tbl)
	if not tbl.name then
		error("Binary definition must include a 'name' field")
	end

	if not tbl.targets then
		error(("Binary '%s' must specify targets"):format(tbl.name))
	end

	for target_name, target_config in pairs(tbl.targets) do
		if not target_config.target then
			error(("Binary '%s' target '%s' must specify a target"):format(tbl.name, target_name))
		end

		local compiler_name = target_config.compiler or "gcc"
		if not M.compilers[compiler_name] then
			forge.log.warn(("Unknown compiler '%s' for binary '%s' target '%s'"):format(compiler_name, tbl.name, target_name))
		end

		if target_config.standard then
			local valid_std = common.validate_standard(target_config.standard)
			if not valid_std then
				forge.log.warn(
					("Invalid C++ standard '%s' for binary '%s' target '%s'"):format(target_config.standard, tbl.name, target_name)
				)
			end
		end
	end

	return true
end

function M.library(tbl)
	validate_library(tbl)

	tbl.is_lib = true
	defined_libraries[tbl.name] = tbl

	forge.log.info(("Defining C++ library '%s' with %d targets"):format(tbl.name, table_length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_library_rules_for_target(tbl, target_name, target_config)
	end
end

function M.binary(tbl)
	validate_binary(tbl)

	tbl.is_lib = false

	forge.log.info(("Defining C++ binary '%s' with %d targets"):format(tbl.name, table_length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_program_rules_for_target(tbl, target_name, target_config)
	end
end

M.executable = M.binary

function table_length(t)
	local count = 0
	for _ in pairs(t) do
		count = count + 1
	end
	return count
end

M.utils = {
	resolve_sources = common.resolve_sources,
	resolve_includes = common.resolve_includes,
	get_host_target = common.get_host_target,
	get_compiler_for_target = common.get_compiler_for_target,
	validate_standard = common.validate_standard,
}

return M
