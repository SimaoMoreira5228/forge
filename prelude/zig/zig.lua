local common = require("@prelude/zig/zig_common.lua")
local compiler = require("@prelude/zig/zig_compiler.lua")

local M = {}

M.predefined_targets = common.predefined_targets
M.build_modes = common.build_modes

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

		if target_config.mode and not common.validate_build_mode(target_config.mode) then
			forge.log.warn(
				("Invalid build mode '%s' for library '%s' target '%s'"):format(target_config.mode, tbl.name, target_name)
			)
		end
	end

	return true
end

local function validate_executable(tbl)
	if not tbl.name then
		error("Executable definition must include a 'name' field")
	end

	if not tbl.targets then
		error(("Executable '%s' must specify targets"):format(tbl.name))
	end

	for target_name, target_config in pairs(tbl.targets) do
		if not target_config.target then
			error(("Executable '%s' target '%s' must specify a target"):format(tbl.name, target_name))
		end

		if target_config.mode and not common.validate_build_mode(target_config.mode) then
			forge.log.warn(
				("Invalid build mode '%s' for executable '%s' target '%s'"):format(target_config.mode, tbl.name, target_name)
			)
		end
	end

	return true
end

local function validate_build_zig(tbl)
	if not tbl.name then
		error("build.zig project must include a 'name' field")
	end

	if not tbl.targets then
		error(("build.zig project '%s' must specify targets"):format(tbl.name))
	end

	if not tbl.outputs or #tbl.outputs == 0 then
		error(("build.zig project '%s' must specify outputs"):format(tbl.name))
	end

	for target_name, target_config in pairs(tbl.targets) do
		if not target_config.target then
			error(("build.zig project '%s' target '%s' must specify a target"):format(tbl.name, target_name))
		end

		if target_config.mode and not common.validate_build_mode(target_config.mode) then
			forge.log.warn(
				("Invalid build mode '%s' for build.zig project '%s' target '%s'"):format(target_config.mode, tbl.name, target_name)
			)
		end
	end

	return true
end

function M.library(tbl)
	validate_library(tbl)

	tbl.is_lib = true
	defined_libraries[tbl.name] = tbl

	forge.log.info(("Defining Zig library '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_library_rules_for_target(tbl, target_name, target_config)
	end
end

function M.executable(tbl)
	validate_executable(tbl)

	tbl.is_lib = false

	forge.log.info(("Defining Zig executable '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_executable_rules_for_target(tbl, target_name, target_config)
	end
end

M.binary = M.executable

function M.build_zig(tbl)
	validate_build_zig(tbl)

	forge.log.info(("Defining build.zig project '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		compiler.define_build_zig_rules_for_target(tbl, target_name, target_config)
	end
end

M.utils = {
	resolve_sources = common.resolve_sources,
	resolve_includes = common.resolve_includes,
	get_host_target = common.get_host_target,
	get_zig_target_string = common.get_zig_target_string,
	validate_build_mode = common.validate_build_mode,
}

return M
