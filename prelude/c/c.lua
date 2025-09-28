local common = require("@prelude/c/c_common.lua")
local compiler = require("@prelude/c/c_compiler.lua")

local M = {}

M.predefined_targets = common.predefined_targets
M.compilers = common.compilers

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
	end

	return true
end

function M.library(tbl)
	validate_library(tbl)

	tbl.is_lib = true
	defined_libraries[tbl.name] = tbl

	forge.log.info(("Defining C library '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_library_rules_for_target(tbl, target_name, target_config)
	end
end

function M.binary(tbl)
	validate_binary(tbl)

	tbl.is_lib = false

	forge.log.info(("Defining C binary '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_program_rules_for_target(tbl, target_name, target_config)
	end
end

M.executable = M.binary

M.utils = {
	resolve_sources = common.resolve_sources,
	resolve_includes = common.resolve_includes,
	get_host_target = common.get_host_target,
	get_compiler_for_target = common.get_compiler_for_target,
}

return M
