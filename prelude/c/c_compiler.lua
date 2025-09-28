local build_common = require("@prelude/build_common.lua")
local common = require("@prelude/c/c_common.lua")
local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

local to_absolute_path = compiler_common.to_absolute_path
local ensure_dir = compiler_common.ensure_dir

function M.define_program_rules_for_target(program_info, target_name, target_config)
	if not build_common.should_build_component(program_info.name, target_name, program_info.dependencies) then
		return
	end

	local program_path = program_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local compiler_name = target_config.compiler or "gcc"
	local compiler_path = target_config.compiler_path or program_info.compiler_path

	local compiler_info = common.get_compiler_for_target(compiler_name, target, compiler_path)

	local out_dir = forge.path.join({
		forge.project.root,
		"forge-out",
		target_common.get_target_directory(target_common.extract_base_target(target_name), target_name),
	})
	ensure_dir(out_dir)

	local output_name = program_info.name

	if forge.config and forge.config.test_mode then
		output_name = output_name .. "_test"
	end

	if target.os == "windows" then
		output_name = output_name .. ".exe"
	end
	local output_path = forge.path.join({ out_dir, output_name })

	local sources
	if program_info.srcs then
		sources = common.resolve_sources(program_info.srcs, program_path)
	else
		local pattern = forge.path.join({ program_path, "**/*.c" })
		sources = forge.fs.glob(pattern)
	end

	if #sources == 0 then
		forge.log.warn(("No C source files found for program '%s'"):format(program_info.name))
		return
	end

	local dep_inputs = {}
	local dep_rules = {}
	local link_libraries = {}
	local library_paths = {}

	if program_info.dependencies then
		for name, details in pairs(program_info.dependencies) do
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
				table.insert(link_libraries, name)
				table.insert(library_paths, dep_out_dir)
			end
		end
	end

	local args = {}

	for _, arg in ipairs(compiler_info.args) do
		table.insert(args, arg)
	end

	for _, src in ipairs(sources) do
		table.insert(args, to_absolute_path(src, program_path))
	end

	table.insert(args, "-o")
	table.insert(args, output_path)

	local build_config = build_common.resolve_build_config(target_config, "c")

	for _, flag in ipairs(build_config.opt_flags) do
		table.insert(args, flag)
	end

	for _, flag in ipairs(build_config.debug_flags) do
		table.insert(args, flag)
	end

	for _, define in ipairs(build_config.defines) do
		local key, value = define:match("([^=]+)=(.+)")
		if key and value then
			table.insert(args, "-D" .. key .. "=" .. value)
		else
			table.insert(args, "-D" .. define)
		end
	end

	if program_info.includes then
		local includes = common.resolve_includes(program_info.includes, program_path)
		for _, include_dir in ipairs(includes) do
			table.insert(args, "-I" .. include_dir)
		end
	end

	if program_info.defines then
		for name, value in pairs(program_info.defines) do
			if value == true or value == "" then
				table.insert(args, "-D" .. name)
			else
				table.insert(args, "-D" .. name .. "=" .. tostring(value))
			end
		end
	end

	for _, lib_path in ipairs(library_paths) do
		table.insert(args, "-L" .. lib_path)
	end

	for _, lib in ipairs(link_libraries) do
		table.insert(args, "-l" .. lib)
	end

	if program_info.rule_dependencies then
		for _, rule_name in ipairs(program_info.rule_dependencies) do
			table.insert(dep_rules, rule_name)
		end
	end

	local system_libs = target_config.system_libs or program_info.system_libs
	if system_libs then
		for _, lib in ipairs(system_libs) do
			table.insert(args, "-l" .. lib)
		end
	end

	if program_info.cflags then
		for _, flag in ipairs(program_info.cflags) do
			table.insert(args, flag)
		end
	end

	if program_info.ldflags then
		for _, flag in ipairs(program_info.ldflags) do
			table.insert(args, flag)
		end
	end

	local inputs = {}
	for _, src in ipairs(sources) do
		table.insert(inputs, to_absolute_path(src, program_path))
	end

	for _, dep_input in ipairs(dep_inputs) do
		table.insert(inputs, dep_input)
	end

	forge.rule({
		name = ("%s-compile-%s"):format(program_info.name, target_name),
		command = compiler_info.command,
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
	local compiler_name = target_config.compiler or "gcc"
	local compiler_path = target_config.compiler_path or library_info.compiler_path

	local compiler_info = common.get_compiler_for_target(compiler_name, target, compiler_path)

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

	local sources
	if library_info.srcs then
		sources = common.resolve_sources(library_info.srcs, library_path)
	else
		local pattern = forge.path.join({ library_path, "**/*.c" })
		sources = forge.fs.glob(pattern)
	end

	if #sources == 0 then
		forge.log.warn(("No C source files found for library '%s'"):format(library_info.name))
		return
	end

	local object_files = {}
	local compile_rules = {}

	for i, src in ipairs(sources) do
		local obj_name = forge.path.stem(forge.path.basename(src)) .. ".o"
		local obj_path = forge.path.join({ out_dir, obj_name })
		table.insert(object_files, obj_path)

		local compile_rule_name = ("%s-obj-%d-%s"):format(library_info.name, i, target_name)
		table.insert(compile_rules, compile_rule_name)

		local compile_args = {}
		for _, arg in ipairs(compiler_info.args) do
			table.insert(compile_args, arg)
		end

		table.insert(compile_args, "-c")
		table.insert(compile_args, to_absolute_path(src, library_path))
		table.insert(compile_args, "-o")
		table.insert(compile_args, obj_path)

		local build_config = build_common.resolve_build_config(target_config, "c")

		for _, flag in ipairs(build_config.opt_flags) do
			table.insert(compile_args, flag)
		end

		for _, flag in ipairs(build_config.debug_flags) do
			table.insert(compile_args, flag)
		end

		for _, define in ipairs(build_config.defines) do
			local key, value = define:match("([^=]+)=(.+)")
			if key and value then
				table.insert(compile_args, "-D" .. key .. "=" .. value)
			else
				table.insert(compile_args, "-D" .. define)
			end
		end

		if library_info.includes then
			local includes = common.resolve_includes(library_info.includes, library_path)
			for _, include_dir in ipairs(includes) do
				table.insert(compile_args, "-I" .. include_dir)
			end
		end

		if library_info.defines then
			for name, value in pairs(library_info.defines) do
				if value == true or value == "" then
					table.insert(compile_args, "-D" .. name)
				else
					table.insert(compile_args, "-D" .. name .. "=" .. tostring(value))
				end
			end
		end

		if library_info.cflags then
			for _, flag in ipairs(library_info.cflags) do
				table.insert(compile_args, flag)
			end
		end

		forge.rule({
			name = compile_rule_name,
			command = compiler_info.command,
			args = compile_args,
			inputs = { to_absolute_path(src, library_path) },
			outputs = { obj_path },
		})
	end

	local ar_args = { "rcs", output_path }
	for _, obj in ipairs(object_files) do
		table.insert(ar_args, obj)
	end

	forge.rule({
		name = ("%s-lib-%s"):format(library_info.name, target_name),
		command = "ar",
		args = ar_args,
		inputs = object_files,
		outputs = { output_path },
		dependencies = compile_rules,
	})
end

return M
