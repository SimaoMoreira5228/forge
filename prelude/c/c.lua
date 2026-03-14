local common = require("@prelude/c/c_common.lua")
local compiler = require("@prelude/c/c_compiler.lua")

local M = {}

M.predefined_targets = common.predefined_targets
M.compilers = common.compilers

local function normalize_deps(deps)
	if not deps then
		return {}
	end
	local normalized = {}
	for key, value in pairs(deps) do
		if type(key) == "number" then
			table.insert(normalized, value)
		else
			table.insert(normalized, key)
		end
	end
	return normalized
end

local function normalize_sources(srcs, path)
	if not srcs then
		return {}
	end
	local sources = {}
	for _, src in ipairs(srcs) do
		table.insert(sources, src)
	end
	return sources
end

local function normalize_includes(includes, path)
	if not includes then
		return {}
	end
	local result = {}
	for _, inc in ipairs(includes) do
		table.insert(result, inc)
	end
	return result
end

local function should_build_target(target_name)
	if forge.config.target_filters and #forge.config.target_filters > 0 then
		for _, filter in ipairs(forge.config.target_filters) do
			if filter == target_name then
				return true
			end
		end
		return false
	end
	return true
end

local function validate_library(tbl)
	if not tbl.name then
		error("Library definition must include a 'name' field")
	end

	if not tbl.targets then
		error(("Library '%s' must specify targets"):format(tbl.name))
	end

	for target_name, target_config in pairs(tbl.targets) do
		if not should_build_target(target_name) then
			goto continue
		end
		if not target_config.target then
			error(("Library '%s' target '%s' must specify a target"):format(tbl.name, target_name))
		end
		::continue::
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
		if not should_build_target(target_name) then
			goto continue
		end
		if not target_config.target then
			error(("Binary '%s' target '%s' must specify a target"):format(tbl.name, target_name))
		end
		::continue::
	end

	return true
end

local function register_target(target_name, target_info)
	local triple
	if type(target_info.target) == "table" then
		triple = target_info.target.triple or target_info.target.canonical_name
		if not triple then
			error(("Target '%s' is a table but missing triple"):format(target_name))
		end
	else
		triple = target_info.target
	end

	forge.graph:target {
		name = target_name,
		triple = triple,
	}
end

local function define_library_for_target(library_info, target_name, target_config)
	if not should_build_target(target_name) then
		return
	end

	register_target(target_name, target_config)

	local sources = normalize_sources(library_info.srcs, library_info.path)
	local include_dirs = normalize_includes(library_info.includes, library_info.path)
	local deps = normalize_deps(library_info.dependencies)

	forge.graph:library {
		name = library_info.name,
		target = target_name,
		sources = sources,
		include_dirs = include_dirs,
		defines = library_info.defines,
		cflags = library_info.cflags,
		deps = deps,
	}

	compiler.define_library_rules_for_target(library_info, target_name, target_config)
end

local function define_binary_for_target(binary_info, target_name, target_config)
	if not should_build_target(target_name) then
		return
	end

	register_target(target_name, target_config)

	local sources = normalize_sources(binary_info.srcs, binary_info.path)
	local include_dirs = normalize_includes(binary_info.includes, binary_info.path)
	local deps = normalize_deps(binary_info.dependencies)

	forge.graph:binary {
		name = binary_info.name,
		target = target_name,
		sources = sources,
		include_dirs = include_dirs,
		defines = binary_info.defines,
		cflags = binary_info.cflags,
		ldflags = binary_info.ldflags,
		system_libs = target_config.system_libs or binary_info.system_libs,
		deps = deps,
	}

	compiler.define_program_rules_for_target(binary_info, target_name, target_config)
end

function M.library(tbl)
	validate_library(tbl)

	forge.log.info(("Defining C library '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		define_library_for_target(tbl, target_name, target_config)
	end
end

function M.binary(tbl)
	validate_binary(tbl)

	forge.log.info(("Defining C binary '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		define_binary_for_target(tbl, target_name, target_config)
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
