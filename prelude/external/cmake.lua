local build_common = require("@prelude/build_common.lua")

local M = {}

local function validate_cmake_project(tbl)
	if not tbl.name then
		error("CMake project must include a 'name' field")
	end

	if not tbl.targets then
		error(("CMake project '%s' must specify targets"):format(tbl.name))
	end

	if not tbl.outputs or forge.table.length(tbl.outputs) == 0 then
		error(("CMake project '%s' must specify outputs"):format(tbl.name))
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
	validate_cmake_project(tbl)

	forge.log.info(("Defining CMake project '%s' with %d targets"):format(tbl.name, forge.table.length(tbl.targets)))

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

	local build_dir = tbl.build_dir or "build"
	if not forge.path.is_absolute(build_dir) then
		build_dir = forge.path.join({ project_root, build_dir })
	end

	local target_build_dir = forge.path.join({ build_dir, target_name })

	local cmake_command = tbl.cmake_command or "cmake"
	local cmake_lists = forge.path.join({ project_root, "CMakeLists.txt" })

	local configure_args = { "-S", project_root, "-B", target_build_dir }

	if tbl.configure_args then
		for _, arg in ipairs(tbl.configure_args) do
			table.insert(configure_args, arg)
		end
	end
	if target_config.configure_args then
		for _, arg in ipairs(target_config.configure_args) do
			table.insert(configure_args, arg)
		end
	end

	local build_args = { "--build", target_build_dir }

	if tbl.cmake_targets then
		for _, cmake_target in ipairs(tbl.cmake_targets) do
			table.insert(build_args, "--target")
			table.insert(build_args, cmake_target)
		end
	end

	if tbl.build_args then
		for _, arg in ipairs(tbl.build_args) do
			table.insert(build_args, arg)
		end
	end
	if target_config.build_args then
		for _, arg in ipairs(target_config.build_args) do
			table.insert(build_args, arg)
		end
	end

	local inputs = { cmake_lists }

	if tbl.srcs then
		for _, src in ipairs(tbl.srcs) do
			local src_path = to_absolute_path(src, project_root)
			table.insert(inputs, src_path)
		end
	else
		local patterns = {
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

	forge.rule({
		name = ("%s-configure-%s"):format(tbl.name, target_name),
		command = cmake_command,
		args = configure_args,
		inputs = { cmake_lists },
		outputs = { forge.path.join({ target_build_dir, "CMakeCache.txt" }) },
		dependencies = tbl.dependencies or {},
		workdir = project_root,
	})

	forge.rule({
		name = ("%s-build-%s"):format(tbl.name, target_name),
		command = cmake_command,
		args = build_args,
		inputs = inputs,
		outputs = outputs,
		dependencies = { ("%s-configure-%s"):format(tbl.name, target_name) },
		workdir = project_root,
	})

	forge.log.info(
		("CMake project '%s' target '%s' configured in %s with %d outputs"):format(
			tbl.name,
			target_name,
			target_build_dir,
			forge.table.length(outputs)
		)
	)
end

return M
