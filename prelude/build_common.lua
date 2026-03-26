local M = {}

function M.get_out_dir(target_name, exec_cfg)
	local profile_name = "debug"
	if forge and forge.profile then
		profile_name = forge.profile:name()
	end

	if exec_cfg == "host" or exec_cfg == "exec" then
		local host_platform = forge.project.host_platform or "host"
		return forge.path.join({
			forge.project.root,
			"forge-out",
			"host-" .. host_platform,
			profile_name,
		})
	end

	local base_out = forge.path.join({
		forge.project.root,
		"forge-out",
		target_name,
		profile_name,
	})

	if forge.config and forge.config.test_mode then
		return forge.path.join({ base_out, "test" })
	end

	return base_out
end

M.optimization_levels = {
	debug = {
		level = 0,
		description = "No optimizations, maximum debugging info",
		rust_flags = { "-C", "opt-level=0" },
		c_flags = { "-O0", "-g" },
		cpp_flags = { "-O0", "-g" },
		zig_mode = "Debug",
	},
	basic = {
		level = 1,
		description = "Basic optimizations",
		rust_flags = { "-C", "opt-level=1" },
		c_flags = { "-O1", "-g" },
		cpp_flags = { "-O1", "-g" },
		zig_mode = "Debug",
	},
	some = {
		level = 2,
		description = "Some optimizations",
		rust_flags = { "-C", "opt-level=2" },
		c_flags = { "-O2" },
		cpp_flags = { "-O2" },
		zig_mode = "ReleaseSafe",
	},
	full = {
		level = 3,
		description = "Full optimizations",
		rust_flags = { "-C", "opt-level=3" },
		c_flags = { "-O3" },
		cpp_flags = { "-O3" },
		zig_mode = "ReleaseFast",
	},
	size = {
		level = "s",
		description = "Optimize for size",
		rust_flags = { "-C", "opt-level=s" },
		c_flags = { "-Os" },
		cpp_flags = { "-Os" },
		zig_mode = "ReleaseSmall",
	},
	size_aggressive = {
		level = "z",
		description = "Aggressively optimize for size",
		rust_flags = { "-C", "opt-level=z" },
		c_flags = { "-Oz" },
		cpp_flags = { "-Oz" },
		zig_mode = "ReleaseSmall",
	},
}

M.debug_levels = {
	none = {
		level = 0,
		description = "No debug information",
		rust_flags = { "-C", "debuginfo=0" },
		c_flags = {},
		cpp_flags = {},
	},
	lines = {
		level = 1,
		description = "Line number information only",
		rust_flags = { "-C", "debuginfo=1" },
		c_flags = { "-g1" },
		cpp_flags = { "-g1" },
	},
	full = {
		level = 2,
		description = "Full debug information",
		rust_flags = { "-C", "debuginfo=2" },
		c_flags = { "-g" },
		cpp_flags = { "-g" },
	},
}

M.build_profiles = {
	debug = {
		optimization = "debug",
		debug_info = "full",
		defines = { "DEBUG=1" },
		description = "Development build with debugging",
	},
	dev = {
		optimization = "basic",
		debug_info = "full",
		defines = { "DEBUG=1" },
		description = "Development build with basic optimization",
	},
	release = {
		optimization = "full",
		debug_info = "lines",
		defines = { "NDEBUG=1", "RELEASE=1" },
		description = "Production release build",
	},
	release_debug = {
		optimization = "full",
		debug_info = "full",
		defines = { "NDEBUG=1", "RELEASE=1" },
		description = "Release build with full debug info",
	},
	size = {
		optimization = "size",
		debug_info = "none",
		defines = { "NDEBUG=1" },
		description = "Size-optimized build",
	},
}

M.dependency_types = {
	rust_library = {
		file_extension = ".rlib",
		search_patterns = { "src/**/*.rs" },
		link_type = "rust_extern",
	},
	c_library = {
		file_extension = ".a",
		search_patterns = { "**/*.c", "**/*.h" },
		link_type = "static",
	},
	cpp_library = {
		file_extension = ".a",
		search_patterns = { "**/*.cpp", "**/*.hpp", "**/*.cc", "**/*.cxx" },
		link_type = "static",
	},
	dynamic_library = {
		file_extension = ".so",
		link_type = "dynamic",
	},
}

function M.resolve_build_config(target_config, language)
	local profile = forge.profile
	if not profile then
		error("forge.profile is not available in the Lua environment")
	end

	local build_config = {
		profile = profile:name(),
		defines = profile:defines() or {},
		is_release = profile:name() ~= "debug" and profile:name() ~= "dev",
	}

	if target_config.defines then
		for k, v in pairs(target_config.defines) do
			build_config.defines[k] = v
		end
	end

	local opt_flags = {}
	local debug_flags = {}
	local opt_level = profile:opt_level()
	local has_debug = profile:debug()

	if language == "c" or language == "cpp" then
		if opt_level == 0 then
			table.insert(opt_flags, "-O0")
		elseif opt_level == 1 then
			table.insert(opt_flags, "-O1")
		elseif opt_level == 2 then
			table.insert(opt_flags, "-O2")
		elseif opt_level >= 3 then
			table.insert(opt_flags, "-O3")
		end

		if has_debug then
			table.insert(debug_flags, "-g")
		end
		if profile:lto() then
			table.insert(opt_flags, "-flto")
		end

	elseif language == "d" then
		if opt_level > 0 then
			table.insert(opt_flags, "-O")
		end
		if has_debug then
			table.insert(debug_flags, "-g")
		end
		-- D LTO flag standard check, leaving empty for now but can add if using LDC
		
	elseif language == "zig" then
		if opt_level == 0 then
			table.insert(opt_flags, "-O")
			table.insert(opt_flags, "Debug")
		elseif opt_level == 3 then
			table.insert(opt_flags, "-O")
			table.insert(opt_flags, "ReleaseFast")
		else
			table.insert(opt_flags, "-O")
			table.insert(opt_flags, "ReleaseSafe")
		end
	end

	for _, san in ipairs(profile:sanitizers() or {}) do
		if language == "c" or language == "cpp" or language == "d" then
			table.insert(opt_flags, "-fsanitize=" .. san)
		end
	end

	build_config.opt_flags = opt_flags
	build_config.debug_flags = debug_flags
	build_config.profile_compiler_flags = profile:compiler_flags() or {}
	build_config.profile_linker_flags = profile:linker_flags() or {}

	return build_config
end

function M.validate_dependency_type(dep_type)
	return M.dependency_types[dep_type] ~= nil
end

function M.get_dependency_info(dep_type)
	return M.dependency_types[dep_type]
end

M.source_patterns = {
	rust = { "src/**/*.rs", "**/*.rs" },
	c = { "src/**/*.c", "**/*.c" },
	cpp = { "src/**/*.cpp", "src/**/*.cxx", "src/**/*.cc", "**/*.cpp", "**/*.cxx", "**/*.cc" },
	zig = { "src/**/*.zig", "**/*.zig" },
	header = { "src/**/*.h", "src/**/*.hpp", "**/*.h", "**/*.hpp" },
}

function M.get_source_patterns(language)
	return M.source_patterns[language] or {}
end

local function has_value(list, value)
	if not list then
		return false
	end
	for _, item in ipairs(list) do
		if item == value then
			return true
		end
	end
	return false
end

function M.should_build_target(target_name)
	local filters = forge.config and forge.config.target_filters or nil
	if not filters or #filters == 0 then
		return true
	end
	return has_value(filters, target_name)
end

function M.should_build_component(component_name, target_name, _dependencies)
	local component_filters = forge.config and forge.config.component_filters or nil
	if component_filters and #component_filters > 0 and not has_value(component_filters, component_name) then
		return false
	end

	if target_name then
		return M.should_build_target(target_name)
	end

	return true
end

return M
