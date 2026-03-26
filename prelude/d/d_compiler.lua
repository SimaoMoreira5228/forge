local build_common = require("@prelude/build_common.lua")
local common = require("@prelude/d/d_common.lua")
local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

local to_absolute_path = compiler_common.to_absolute_path
local ensure_dir = compiler_common.ensure_dir

function M.define_program_rules_for_target(program_info, target_name, target_config)
	local program_path = program_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local compiler_name = target_config.compiler or "gdc"
	local compiler_path = target_config.compiler_path or program_info.compiler_path

	local compiler_info = common.get_compiler_for_target(compiler_name, target, compiler_path)

	local out_dir = build_common.get_out_dir(target_name)
	ensure_dir(out_dir)

	local output_name = program_info.name
	if target.os == "windows" then
		output_name = output_name .. ".exe"
	end
	local output_path = forge.path.join({ out_dir, output_name })

	local sources = program_info.srcs and common.resolve_sources(program_info.srcs, program_path)
		or forge.fs.glob(forge.path.join({ program_path, "**/*.d" }))

	local args = {}
	for _, arg in ipairs(compiler_info.args) do
		table.insert(args, arg)
	end

	for _, src in ipairs(sources) do
		table.insert(args, to_absolute_path(src, program_path))
	end

	table.insert(args, "-of=" .. output_path)

	if program_info.dflags then
		for _, flag in ipairs(program_info.dflags) do
			table.insert(args, flag)
		end
	end

	local inputs = {}
	for _, src in ipairs(sources) do
		table.insert(inputs, to_absolute_path(src, program_path))
	end

	local dep_rules = {}
	if program_info.dependencies then
		for name, details in pairs(program_info.dependencies) do
			local dep_out_dir = build_common.get_out_dir(target_name)
			local dep_output_path = forge.path.join({ dep_out_dir, "lib" .. name .. ".a" })

			table.insert(dep_rules, ("%s-lib-%s"):format(name, target_name))
			table.insert(inputs, dep_output_path)

			if compiler_name == "ldc" or compiler_name == "dmd" then
				table.insert(args, "-L-L" .. dep_out_dir)
				table.insert(args, "-L-l" .. name)
			else
				table.insert(args, "-L" .. dep_out_dir)
				table.insert(args, "-l" .. name)
			end
		end
	end

	local system_libs = target_config.system_libs or program_info.system_libs
	if system_libs then
		for _, lib in ipairs(system_libs) do
			if compiler_name == "ldc" or compiler_name == "dmd" then
				table.insert(args, "-L-l" .. lib)
			else
				table.insert(args, "-l" .. lib)
			end
		end
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
	local library_path = library_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local compiler_name = target_config.compiler or "gdc"

	local out_dir = build_common.get_out_dir(target_name)
	ensure_dir(out_dir)

	local output_name = "lib" .. library_info.name .. ".a"
	local output_path = forge.path.join({ out_dir, output_name })

	local sources = library_info.srcs and common.resolve_sources(library_info.srcs, library_path)
		or forge.fs.glob(forge.path.join({ library_path, "**/*.d" }))

	local compiler_info = common.get_compiler_for_target(compiler_name, target)

	local args = { "-lib" }
	for _, src in ipairs(sources) do
		table.insert(args, to_absolute_path(src, library_path))
	end
	table.insert(args, "-of" .. output_path)

	local inputs = {}
	for _, src in ipairs(sources) do
		table.insert(inputs, to_absolute_path(src, library_path))
	end

	forge.rule({
		name = ("%s-lib-%s"):format(library_info.name, target_name),
		command = compiler_info.command,
		args = args,
		inputs = inputs,
		outputs = { output_path },
	})
end

return M
