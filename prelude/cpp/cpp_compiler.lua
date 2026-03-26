local build_common = require("@prelude/build_common.lua")
local common = require("@prelude/cpp/cpp_common.lua")
local compiler_common = require("@prelude/compiler_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

local to_absolute_path = compiler_common.to_absolute_path
local ensure_dir = compiler_common.ensure_dir

function M.define_module_bmi_rules(sources, program_info, target_name, target_config, out_dir, compiler_info)
	local program_path = program_info.path or forge.project.root
	local include_dirs = program_info.includes or {}

	local resolved_sources = sources

	if not common.has_modules(resolved_sources) then
		return {}
	end

	local module_info = common.scan_modules(resolved_sources, include_dirs)
	local modules = module_info.modules

	local bmi_rules = {}

	for module_name, module_data in pairs(modules) do
		local source_file = module_data.source_file
		local bmi_name = module_name .. (compiler_info.id == "msvc" and ".ifc" or ".pcm")
		local bmi_path = out_dir .. "/" .. bmi_name
		local obj_name = module_name .. ".o"
		local obj_path = out_dir .. "/" .. obj_name

		local bmi_args = {}
		for _, arg in ipairs(compiler_info.args) do
			table.insert(bmi_args, arg)
		end

		if compiler_info.id == "msvc" then
			table.insert(bmi_args, "/c")
			table.insert(bmi_args, "/interface")
			table.insert(bmi_args, "/ifcOutput")
			table.insert(bmi_args, bmi_path)
			table.insert(bmi_args, "/Fo" .. obj_path)
			table.insert(bmi_args, to_absolute_path(source_file, program_path))
			for _, inc in ipairs(include_dirs) do
				local resolved_inc = to_absolute_path(inc, program_path)
				table.insert(bmi_args, "/I" .. resolved_inc)
			end
		else
			table.insert(bmi_args, "-std=c++20")
			table.insert(bmi_args, "-fmodules")
			table.insert(bmi_args, "-c")
			table.insert(bmi_args, "-Xclang")
			table.insert(bmi_args, "-emit-module-interface")
			table.insert(bmi_args, "-fmodule-name=" .. module_name)
			table.insert(bmi_args, "-o")
			table.insert(bmi_args, bmi_path)
			table.insert(bmi_args, to_absolute_path(source_file, program_path))
			for _, inc in ipairs(include_dirs) do
				local resolved_inc = to_absolute_path(inc, program_path)
				table.insert(bmi_args, "-I" .. resolved_inc)
			end
		end

		local bmi_rule_name = ("%s-bmi-%s"):format(module_name, target_name)

		forge.rule({
			name = bmi_rule_name,
			command = compiler_info.command,
			args = bmi_args,
			inputs = { source_file },
			outputs = (compiler_info.id == "msvc") and { bmi_path, obj_path } or { bmi_path },
			env = compiler_info.env,
		})

		local obj_args = {}
		for _, arg in ipairs(compiler_info.args) do
			table.insert(obj_args, arg)
		end

		table.insert(obj_args, "-std=c++20")
		table.insert(obj_args, "-fmodules")
		table.insert(obj_args, "-c")
		table.insert(obj_args, "-fmodule-name=" .. module_name)
		table.insert(obj_args, "-o")
		table.insert(obj_args, obj_path)
		table.insert(obj_args, to_absolute_path(source_file, program_path))

		for _, inc in ipairs(include_dirs) do
			local resolved_inc = to_absolute_path(inc, program_path)
			table.insert(obj_args, "-I" .. resolved_inc)
		end

		local obj_rule_name = ("%s-obj-%s"):format(module_name, target_name)

		if compiler_info.id ~= "msvc" then
			forge.rule({
				name = obj_rule_name,
				command = compiler_info.command,
				args = obj_args,
				inputs = { source_file },
				outputs = { obj_path },
				dependencies = { bmi_rule_name },
			})
		end

		table.insert(bmi_rules, {
			rule_name = bmi_rule_name,
			obj_rule_name = (compiler_info.id == "msvc") and bmi_rule_name or obj_rule_name,
			module_name = module_name,
			bmi_path = bmi_path,
			obj_path = obj_path,
		})

		forge.log.info(("Generated BMI rule for module '%s'"):format(module_name))
	end

	return bmi_rules
end

function M.define_program_rules_for_target(program_info, target_name, target_config)
	if not program_info.srcs then
		return
	end

	local program_path = program_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local compiler_name = target_config.compiler or "gcc"
	local standard = common.validate_standard(target_config.standard or program_info.standard)
	local compiler_path = target_config.compiler_path or program_info.compiler_path

	local compiler_info = common.get_compiler_for_target(compiler_name, target, standard, compiler_path)

	local out_dir = build_common.get_out_dir(target_name, program_info.exec_cfg)
	ensure_dir(out_dir)

	local output_name = program_info.name
	if forge.config and forge.config.test_mode and not program_info.is_explicit_test then
		output_name = output_name .. "_t_bin"
	end
	if target.os == "windows" then
		output_name = output_name .. ".exe"
	end
	local output_path = forge.path.join({ out_dir, output_name })

	local sources = {}
	if program_info.srcs then
		local resolved = common.resolve_sources(program_info.srcs, program_path)
		sources = resolved or {}
		if #sources == 0 and program_info.srcs[1] then
			for _, src in ipairs(program_info.srcs) do
				local abs_src = program_path .. "/" .. src
				if forge.fs.exists(abs_src) then
					table.insert(sources, abs_src)
				end
			end
		end
	else
		local patterns = { "**/*.cpp", "**/*.cxx", "**/*.cc", "**/*.C" }
		for _, pattern in ipairs(patterns) do
			local pattern_path = forge.path.join({ program_path, pattern })
			local files = forge.fs.glob(pattern_path)
			for _, file in ipairs(files) do
				table.insert(sources, file)
			end
		end
	end

	if #sources == 0 then
		return
	end

	local bmi_rules = {}
	if program_info.enable_modules ~= false then
		local has_mods = common.has_modules(sources)
		forge.log.info(("Has modules: %s"):format(tostring(has_mods)))
		if has_mods then
			bmi_rules = M.define_module_bmi_rules(sources, program_info, target_name, target_config, out_dir, compiler_info)
		end
	end

	local dep_inputs = {}
	local dep_rules = {}
	local link_libraries = {}
	local out_dir = build_common.get_out_dir(target_name, target_config)
	local library_paths = { out_dir }

	if program_info.dependencies then
		for name, details in pairs(program_info.dependencies) do
			if details.path then
				local dep_out_dir = forge.path.join({
					forge.project.root,
					"forge-out",
					target_name,
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

	local non_module_sources = sources
	if #bmi_rules > 0 then
		non_module_sources = common.filter_out_module_sources(sources)
		forge.log.info(("Filtered module sources, compiling %d files"):format(#non_module_sources))
	end

	if #bmi_rules > 0 then
		if compiler_info.id == "msvc" then
			for _, bmi in ipairs(bmi_rules) do
				table.insert(args, "/reference")
				table.insert(args, bmi.module_name .. "=" .. bmi.bmi_path)
			end
			table.insert(args, "/I" .. out_dir)
		else
			table.insert(args, "-std=c++20")
			table.insert(args, "-fmodules")
			for _, bmi in ipairs(bmi_rules) do
				table.insert(args, "-fmodule-file=" .. bmi.module_name .. "=" .. bmi.bmi_path)
			end
			table.insert(args, "-I" .. out_dir)
		end
	end

	for _, src in ipairs(non_module_sources) do
		table.insert(args, to_absolute_path(src, program_path))
	end

	for _, bmi in ipairs(bmi_rules) do
		table.insert(args, bmi.obj_path)
	end

	if compiler_info.id == "msvc" then
		table.insert(args, "/Fe" .. output_path)
	else
		table.insert(args, "-o")
		table.insert(args, output_path)
	end

	local build_config = build_common.resolve_build_config(target_config, "cpp")

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
			if compiler_info.id == "msvc" then
				table.insert(args, "/I" .. include_dir)
			else
				table.insert(args, "-I" .. include_dir)
			end
		end
	end

	if program_info.defines then
		local pfx = (compiler_info.id == "msvc") and "/D" or "-D"
		for name, value in pairs(program_info.defines) do
			if value == true or value == "" then
				table.insert(args, pfx .. name)
			else
				table.insert(args, pfx .. name .. "=" .. tostring(value))
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

	if program_info.cxxflags then
		for _, flag in ipairs(program_info.cxxflags) do
			table.insert(args, flag)
		end
	end

	if program_info.ldflags then
		for _, flag in ipairs(program_info.ldflags) do
			table.insert(args, flag)
		end
	end

	local inputs = {}
	for _, src in ipairs(non_module_sources) do
		table.insert(inputs, to_absolute_path(src, program_path))
	end
	for _, dep_input in ipairs(dep_inputs) do
		table.insert(inputs, dep_input)
	end
	for _, bmi in ipairs(bmi_rules) do
		table.insert(inputs, bmi.obj_path)
	end

	local dependencies = dep_rules
	for _, bmi in ipairs(bmi_rules) do
		table.insert(dependencies, bmi.rule_name)
		table.insert(dependencies, bmi.obj_rule_name)
	end

	forge.rule({
		name = ("%s-compile-%s"):format(program_info.name, target_name),
		command = compiler_info.command,
		args = args,
		inputs = inputs,
		outputs = { output_path },
		dependencies = dependencies,
		env = compiler_info.env,
	})
end

function M.define_library_rules_for_target(library_info, target_name, target_config)
	local library_path = library_info.path or forge.project.root
	local target = target_config.target or common.get_host_target()
	local compiler_name = target_config.compiler or "g++"
	local standard = common.validate_standard(target_config.standard or library_info.standard)
	local compiler_path = target_config.compiler_path or library_info.compiler_path

	local compiler_info = common.get_compiler_for_target(compiler_name, target, standard, compiler_path)

	local out_dir = build_common.get_out_dir(target_name, library_info.exec_cfg)
	ensure_dir(out_dir)

	local output_name = "lib" .. library_info.name .. ".a"
	local output_path = forge.path.join({ out_dir, output_name })

	local sources
	if library_info.srcs then
		sources = common.resolve_sources(library_info.srcs, library_path)
	else
		sources = {}
		local patterns = { "**/*.cpp", "**/*.cxx", "**/*.cc", "**/*.C" }
		for _, pattern in ipairs(patterns) do
			local pattern_path = forge.path.join({ library_path, pattern })
			local files = forge.fs.glob(pattern_path)
			for _, file in ipairs(files) do
				table.insert(sources, file)
			end
		end
	end

	if #sources == 0 then
		forge.log.warn(("No C++ source files found for library '%s'"):format(library_info.name))
		return
	end

	local bmi_rules = {}
	if library_info.enable_modules ~= false then
		bmi_rules = M.define_module_bmi_rules(sources, library_info, target_name, target_config, out_dir, compiler_info)
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

		if compiler_info.id == "msvc" then
			table.insert(compile_args, "/c")
			table.insert(compile_args, to_absolute_path(src, library_path))
			table.insert(compile_args, "/Fo" .. obj_path)
		else
			table.insert(compile_args, "-c")
			table.insert(compile_args, to_absolute_path(src, library_path))
			table.insert(compile_args, "-o")
			table.insert(compile_args, obj_path)
		end

		local build_config = build_common.resolve_build_config(target_config, "cpp")

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
				if compiler_info.id == "msvc" then
					table.insert(compile_args, "/I" .. include_dir)
				else
					table.insert(compile_args, "-I" .. include_dir)
				end
			end
		end

		if library_info.defines then
			local pfx = (compiler_info.id == "msvc") and "/D" or "-D"
			for name, value in pairs(library_info.defines) do
				if value == true or value == "" then
					table.insert(compile_args, pfx .. name)
				else
					table.insert(compile_args, pfx .. name .. "=" .. tostring(value))
				end
			end
		end

		if library_info.cxxflags then
			for _, flag in ipairs(library_info.cxxflags) do
				table.insert(compile_args, flag)
			end
		end

		forge.rule({
			name = compile_rule_name,
			inputs = { to_absolute_path(src, library_path) },
			outputs = { obj_path },
			env = compiler_info.env,
		})
	end

	local ar_args = {}
	if compiler_info.id == "msvc" then
		table.insert(ar_args, "/OUT:" .. output_path)
	else
		table.insert(ar_args, "rcs")
		table.insert(ar_args, output_path)
	end

	for _, obj in ipairs(object_files) do
		table.insert(ar_args, obj)
	end

	local ar_command = (compiler_info.id == "msvc") and "lib" or "ar"
	if forge.path.is_absolute(compiler_info.command) then
		local compiler_bin_dir = forge.path.dirname(compiler_info.command)
		local candidate_ar = forge.path.join({ compiler_bin_dir, ar_command })
		if forge.fs.exists(candidate_ar) then
			ar_command = candidate_ar
		end
	end

	forge.rule({
		name = ("%s-lib-%s"):format(library_info.name, target_name),
		command = ar_command,
		inputs = object_files,
		outputs = { output_path },
		dependencies = compile_rules,
		env = compiler_info.env,
	})
end

return M
