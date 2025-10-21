local build_common = require("@prelude/build_common.lua")

local M = {}

local function validate_make_project(tbl)
	if not tbl.name then
		error("Make project must include a 'name' field")
	end

	if not tbl.targets then
		error(("Make project '%s' must specify targets"):format(tbl.name))
	end

	if not tbl.outputs or forge.table.length(tbl.outputs) == 0 then
		error(("Make project '%s' must specify outputs"):format(tbl.name))
	end

	return true
end

local function to_absolute_path(path, base_dir)
	if forge.path.is_absolute(path) then
		return path
	end
	return forge.path.join({ base_dir, path })
end

function M.build(tbl)
	validate_make_project(tbl)

	forge.log.info(("Defining Make project '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

	for target_name, target_config in pairs(tbl.targets) do
		M.build_for_target(tbl, target_name, target_config)
	end
end

function M.build_for_target(tbl, target_name, target_config)
	if not build_common.should_build_component(tbl.name, target_name, tbl.dependencies) then
		return
	end

	local project_root = tbl.source_dir or forge.project.root
	if not forge.path.is_absolute(project_root) then
		project_root = forge.path.join({ forge.project.root, project_root })
	end

	local make_command = tbl.make_command or "make"
	local makefile = tbl.makefile or "Makefile"
	local makefile_path = to_absolute_path(makefile, project_root)

	local args = { "-f", makefile_path }

	local jobs = target_config.jobs or tbl.jobs
	if jobs then
		table.insert(args, "-j")
		table.insert(args, tostring(jobs))
	end

	if tbl.make_args then
		for _, arg in ipairs(tbl.make_args) do
			table.insert(args, arg)
		end
	end
	if target_config.make_args then
		for _, arg in ipairs(target_config.make_args) do
			table.insert(args, arg)
		end
	end

	local make_targets = tbl.make_targets or { "all" }
	for _, make_target in ipairs(make_targets) do
		table.insert(args, make_target)
	end

	local inputs = { makefile_path }

	if tbl.srcs then
		for _, src in ipairs(tbl.srcs) do
			local src_path = to_absolute_path(src, project_root)
			table.insert(inputs, src_path)
		end
	else
		local patterns = {
			"*.c",
			"*.cpp",
			"*.cc",
			"*.cxx",
			"*.h",
			"*.hpp",
			"src/**/*.c",
			"src/**/*.cpp",
			"src/**/*.cc",
			"src/**/*.cxx",
			"src/**/*.h",
			"src/**/*.hpp",
			"include/**/*.h",
			"include/**/*.hpp",
		}
		for _, pattern in ipairs(patterns) do
			local full_pattern = forge.path.join({ project_root, pattern })
			local discovered = forge.fs.glob(full_pattern)
			for _, src in ipairs(discovered) do
				table.insert(inputs, src)
			end
		end
	end

	local outputs = {}
	for _, output in ipairs(tbl.outputs) do
		local output_path = to_absolute_path(output, project_root)
		table.insert(outputs, output_path)
	end

	forge.log.info(
		("Make project '%s' target '%s' with %d outputs"):format(tbl.name, target_name, forge.table.length(outputs))
	)

	forge.rule({
		name = ("%s-build-%s"):format(tbl.name, target_name),
		command = make_command,
		args = args,
		inputs = inputs,
		outputs = outputs,
		dependencies = tbl.dependencies or {},
		workdir = project_root,
	})
end

function M.clean(tbl)
	if not tbl.name then
		error("Make clean must include a 'name' field")
	end

	local project_root = tbl.source_dir or forge.project.root
	if not forge.path.is_absolute(project_root) then
		project_root = forge.path.join({ forge.project.root, project_root })
	end

	local make_command = tbl.make_command or "make"
	local makefile = tbl.makefile or "Makefile"
	local makefile_path = to_absolute_path(makefile, project_root)

	local args = { "-f", makefile_path, "clean" }

	forge.rule({
		name = tbl.name .. "-clean",
		command = make_command,
		args = args,
		inputs = { makefile_path },
		outputs = {},
		dependencies = {},
		workdir = project_root,
	})
end

return M
