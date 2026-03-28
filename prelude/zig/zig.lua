local common = require("@prelude/zig/zig_common.lua")
local compiler = require("@prelude/zig/zig_compiler.lua")
local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = common.predefined_targets
M.build_modes = common.build_modes

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
		if not target_config.target then
			error(("Library '%s' target '%s' must specify a target"):format(tbl.name, target_name))
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
	end

	return true
end

local function register_target(target_name, target_info)
	local triple
	if type(target_info.target) == "table" then
		triple = target_info.target.triple or target_info.target.canonical_name
		if not triple then
			error(("Target '%s' is a table but missing triple or canonical_name"):format(target_name))
		end
	else
		triple = target_info.target
	end

	forge.graph.target({
		name = target_name,
		triple = triple,
	})
end

local function define_library_for_target(library_info, target_name, target_config)
	if not should_build_target(target_name) then
		return
	end

	register_target(target_name, target_config)

	local sources = normalize_sources(library_info.srcs, library_info.path)
	local include_dirs = normalize_includes(library_info.includes, library_info.path)
	local deps = normalize_deps(library_info.dependencies)

	forge.graph.library({
		name = library_info.name,
		target = target_name,
		sources = sources,
		include_dirs = include_dirs,
		defines = library_info.defines,
		cflags = library_info.zig_flags,
		deps = deps,
	})

	compiler.define_library_rules_for_target(library_info, target_name, target_config)
end

local function define_executable_for_target(executable_info, target_name, target_config)
	if not should_build_target(target_name) then
		return
	end

	register_target(target_name, target_config)

	local sources = normalize_sources(executable_info.srcs, executable_info.path)
	local include_dirs = normalize_includes(executable_info.includes, executable_info.path)
	local deps = normalize_deps(executable_info.dependencies)

	forge.graph.binary({
		name = executable_info.name,
		target = target_name,
		sources = sources,
		include_dirs = include_dirs,
		defines = executable_info.defines,
		cflags = executable_info.zig_flags,
		ldflags = executable_info.ldflags,
		system_libs = target_config.system_libs or executable_info.system_libs,
		deps = deps,
	})

	compiler.define_executable_rules_for_target(executable_info, target_name, target_config)
end

local function define_build_zig_for_target(build_info, target_name, target_config)
	if not should_build_target(target_name) then
		return
	end

	register_target(target_name, target_config)

	local build_path = build_info.path or forge.project.root
	if not forge.path.is_absolute(build_path) then
		build_path = forge.path.join({ forge.project.root, build_path })
	end

	local build_file = build_info.build_file or "build.zig"
	if not forge.path.is_absolute(build_file) then
		build_file = forge.path.join({ build_path, build_file })
	end

	local target = target_config.target or common.get_host_target()
	local build_mode = target_config.mode or "Debug"
	local zig_command = target_config.compiler_path
		or build_info.compiler_path
		or compiler_common.get_configured_tool_binary("zig", "zig", target)
		or "zig"

	if not common.validate_build_mode(build_mode) then
		forge.log.warn(("Invalid build mode '%s' for build.zig '%s', using Debug"):format(build_mode, build_info.name))
		build_mode = "Debug"
	end

	local zig_target = common.get_zig_target_string(target)

	local host_target = common.get_host_target()
	local is_native = (
		target.arch == host_target.arch
		and target.os == host_target.os
		and (target.abi == host_target.abi or not target.abi)
	)

	if is_native then
		zig_target = "native"
	end

	local build_prefix = build_info.prefix or forge.path.join({ build_path, "zig-out" })

	local cache_dir = forge.path.join({ build_path, ".zig-cache" })
	local global_cache_dir = forge.path.join({ forge.project.root, "forge-out/zig-global-cache" })

	local args = {
		"build",
		"-Dtarget=" .. zig_target,
		"-Doptimize=" .. build_mode,
		"--cache-dir",
		cache_dir,
		"--global-cache-dir",
		global_cache_dir,
		"--prefix",
		build_prefix,
	}

	local env = {
		ZIG_GLOBAL_CACHE_DIR = global_cache_dir,
		ZIG_LOCAL_CACHE_DIR = cache_dir,
	}

	if build_info.steps then
		for _, step in ipairs(build_info.steps) do
			table.insert(args, step)
		end
	end

	if build_info.options then
		for key, value in pairs(build_info.options) do
			table.insert(args, "-D" .. key .. "=" .. tostring(value))
		end
	end

	local inputs = { build_file }

	if build_info.srcs then
		local resolved = common.resolve_sources(build_info.srcs, build_path)
		for _, src in ipairs(resolved) do
			table.insert(inputs, src)
		end
	else
		local pattern = forge.path.join({ build_path, "src/**/*.zig" })
		local discovered = forge.fs.glob(pattern)
		for _, src in ipairs(discovered) do
			table.insert(inputs, src)
		end
	end

	if not build_info.outputs or #build_info.outputs == 0 then
		forge.log.error(("build.zig project '%s' must specify outputs"):format(build_info.name))
		return
	end

	local outputs = {}
	for _, output in ipairs(build_info.outputs) do
		if forge.path.is_absolute(output) then
			table.insert(outputs, output)
		else
			table.insert(outputs, forge.path.join({ build_path, output }))
		end
	end

	forge.log.info(("Building %s with build.zig at %s (workdir: %s)"):format(build_info.name, build_file, build_path))

	forge.rule({
		name = ("%s-build-%s"):format(build_info.name, target_name),
		command = zig_command,
		args = args,
		inputs = inputs,
		outputs = outputs,
		env = env,
		dependencies = normalize_deps(build_info.dependencies),
		workdir = build_path,
	})
end

function M.library(tbl)
	validate_library(tbl)

	forge.log.info(("Defining Zig library '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		define_library_for_target(tbl, target_name, target_config)
	end
end

function M.executable(tbl)
	validate_executable(tbl)

	forge.log.info(("Defining Zig executable '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		define_executable_for_target(tbl, target_name, target_config)
	end
end

M.binary = M.executable

function M.build_zig(tbl)
	validate_build_zig(tbl)

	forge.log.info(("Defining build.zig project '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		define_build_zig_for_target(tbl, target_name, target_config)
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
