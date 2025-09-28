local build_common = require("@prelude/build_common.lua")
local common = require("@prelude/zig/zig_common.lua")
local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

local to_absolute_path = compiler_common.to_absolute_path
local ensure_dir = compiler_common.ensure_dir

function M.define_executable_rules_for_target(executable_info, target_name, target_config)
	if not build_common.should_build_component(executable_info.name, target_name, executable_info.dependencies) then
		return
	end

	local executable_path = executable_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local build_mode = target_config.mode or "Debug"
	local zig_command = target_config.compiler_path or executable_info.compiler_path or "zig"

	if not common.validate_build_mode(build_mode) then
		forge.log.warn(("Invalid build mode '%s' for executable '%s', using Debug"):format(build_mode, executable_info.name))
		build_mode = "Debug"
	end

	local zig_target = common.get_zig_target_string(target)

	if compiler_common.is_native_target(target) then
		zig_target = "native"
	end

	local out_dir = forge.path.join({
		forge.project.root,
		"forge-out",
		target_common.get_target_directory(target_common.extract_base_target(target_name), target_name),
	})
	ensure_dir(out_dir)

	local output_name = executable_info.name

	if forge.config and forge.config.test_mode then
		output_name = output_name .. "_test"
	end

	if target.os == "windows" then
		output_name = output_name .. ".exe"
	end
	local output_path = forge.path.join({ out_dir, output_name })

	local main_file = executable_info.main or "src/main.zig"
	main_file = to_absolute_path(main_file, executable_path)

	if not forge.fs.exists(main_file) then
		forge.log.error(("Main file not found: %s"):format(main_file))
		return
	end

	local sources = { main_file }
	if executable_info.srcs then
		local resolved = common.resolve_sources(executable_info.srcs, executable_path)
		for _, src in ipairs(resolved) do
			if src ~= main_file then
				table.insert(sources, src)
			end
		end
	else
		local pattern = forge.path.join({ executable_path, "**/*.zig" })
		local discovered = forge.fs.glob(pattern)
		for _, src in ipairs(discovered) do
			if src ~= main_file then
				table.insert(sources, src)
			end
		end
	end

	local args = {
		"build-exe",
		main_file,
		"-target",
		zig_target,
		"-O" .. build_mode,
		"-femit-bin=" .. output_path,
	}

	if executable_info.module_path then
		local module_path = to_absolute_path(executable_info.module_path, executable_path)
		table.insert(args, "--mod")
		table.insert(args, "root::" .. module_path)
	end

	if executable_info.lib_paths then
		for _, lib_path in ipairs(executable_info.lib_paths) do
			local abs_lib_path = to_absolute_path(lib_path, executable_path)
			table.insert(args, "-L" .. abs_lib_path)
		end
	end

	local dep_inputs = {}
	local dep_rules = {}

	if executable_info.dependencies then
		for name, details in pairs(executable_info.dependencies) do
			if details.path then
				local dep_out_dir = forge.path.join({
					forge.project.root,
					"forge-out",
					target_common.get_target_directory(target_common.extract_base_target(target_name), target_name),
				})
				local dep_output_path = forge.path.join({ dep_out_dir, "lib" .. name .. ".a" })

				local dep_rule_name = ("%s-lib-%s"):format(name, target_name)
				table.insert(dep_rules, dep_rule_name)
				table.insert(dep_inputs, dep_output_path)
				table.insert(args, "-l" .. name)
				table.insert(args, "-L" .. dep_out_dir)
			end
		end
	end

	if executable_info.rule_dependencies then
		for _, rule_name in ipairs(executable_info.rule_dependencies) do
			table.insert(dep_rules, rule_name)
		end
	end

	local system_libs = target_config.system_libs or executable_info.system_libs
	if system_libs then
		for _, lib in ipairs(system_libs) do
			table.insert(args, "-l" .. lib)
		end
	end

	if executable_info.zig_flags then
		for _, flag in ipairs(executable_info.zig_flags) do
			table.insert(args, flag)
		end
	end

	if executable_info.ldflags then
		for _, flag in ipairs(executable_info.ldflags) do
			table.insert(args, "-Wl," .. flag)
		end
	end

	local inputs = {}
	for _, src in ipairs(sources) do
		table.insert(inputs, src)
	end
	for _, dep_input in ipairs(dep_inputs) do
		table.insert(inputs, dep_input)
	end

	forge.rule({
		name = ("%s-compile-%s"):format(executable_info.name, target_name),
		command = zig_command,
		args = args,
		inputs = inputs,
		outputs = { output_path },
		dependencies = dep_rules,
	})
end

function M.define_library_rules_for_target(library_info, target_name, target_config)
	if not build_common.should_target_be_built(target_name) then
		return
	end

	local library_path = library_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local build_mode = target_config.mode or "Debug"
	local zig_command = target_config.compiler_path or library_info.compiler_path or "zig"

	if not common.validate_build_mode(build_mode) then
		forge.log.warn(("Invalid build mode '%s' for library '%s', using Debug"):format(build_mode, library_info.name))
		build_mode = "Debug"
	end

	local zig_target = common.get_zig_target_string(target)

	if compiler_common.is_native_target(target) then
		zig_target = "native"
	end

	local out_dir = forge.path.join({
		forge.project.root,
		"forge-out",
		target_common.get_target_directory(target_common.extract_base_target(target_name), target_name),
	})
	ensure_dir(out_dir)

	local library_base_name = library_info.name

	if forge.config and forge.config.test_mode then
		library_base_name = library_base_name .. "_test"
	end

	local output_name = "lib" .. library_base_name .. ".a"
	local output_path = forge.path.join({ out_dir, output_name })

	local root_file = library_info.root or "src/lib.zig"
	root_file = to_absolute_path(root_file, library_path)

	if not forge.fs.exists(root_file) then
		forge.log.error(("Root file not found: %s"):format(root_file))
		return
	end

	local sources = { root_file }
	if library_info.srcs then
		local resolved = common.resolve_sources(library_info.srcs, library_path)
		for _, src in ipairs(resolved) do
			if src ~= root_file then
				table.insert(sources, src)
			end
		end
	else
		local pattern = forge.path.join({ library_path, "**/*.zig" })
		local discovered = forge.fs.glob(pattern)
		for _, src in ipairs(discovered) do
			if src ~= root_file then
				table.insert(sources, src)
			end
		end
	end

	local args = {
		"build-lib",
		root_file,
		"-target",
		zig_target,
		"-O" .. build_mode,
		"-femit-bin=" .. output_path,
	}

	if library_info.module_path then
		local module_path = to_absolute_path(library_info.module_path, library_path)
		table.insert(args, "--mod")
		table.insert(args, "root::" .. module_path)
	end

	if library_info.zig_flags then
		for _, flag in ipairs(library_info.zig_flags) do
			table.insert(args, flag)
		end
	end

	forge.rule({
		name = ("%s-lib-%s"):format(library_info.name, target_name),
		command = zig_command,
		args = args,
		inputs = sources,
		outputs = { output_path },
		dependencies = {},
	})
end

function M.define_build_zig_rules_for_target(build_info, target_name, target_config)
	if not build_common.should_build_component(build_info.name, target_name, build_info.dependencies) then
		return
	end

	local build_path = build_info.path or forge.project.root
	if not forge.path.is_absolute(build_path) then
		build_path = forge.path.join({ forge.project.root, build_path })
	end

	local build_file = build_info.build_file or "build.zig"
	build_file = to_absolute_path(build_file, build_path)

	local target = target_config.target or common.get_host_target()
	local build_mode = target_config.mode or "Debug"
	local zig_command = target_config.compiler_path or build_info.compiler_path or "zig"

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

	local args = {
		"build",
		"-Dtarget=" .. zig_target,
		"-Doptimize=" .. build_mode,
		"--prefix",
		build_prefix,
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
		local output_path = to_absolute_path(output, build_path)
		table.insert(outputs, output_path)
	end

	forge.log.info(("Building %s with build.zig at %s (workdir: %s)"):format(build_info.name, build_file, build_path))

	forge.rule({
		name = ("%s-build-%s"):format(build_info.name, target_name),
		command = zig_command,
		args = args,
		inputs = inputs,
		outputs = outputs,
		dependencies = build_info.dependencies or {},
		workdir = build_path,
	})
end

return M
