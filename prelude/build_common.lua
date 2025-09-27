local M = {}

M.optimization_levels = {
	debug = {
		level = 0,
		description = "No optimizations, maximum debugging info",
		rust_flags = { "-C", "opt-level=0" },
		c_flags = { "-O0", "-g" },
		cpp_flags = { "-O0", "-g" },
	},
	basic = {
		level = 1,
		description = "Basic optimizations",
		rust_flags = { "-C", "opt-level=1" },
		c_flags = { "-O1", "-g" },
		cpp_flags = { "-O1", "-g" },
	},
	some = {
		level = 2,
		description = "Some optimizations",
		rust_flags = { "-C", "opt-level=2" },
		c_flags = { "-O2" },
		cpp_flags = { "-O2" },
	},
	full = {
		level = 3,
		description = "Full optimizations",
		rust_flags = { "-C", "opt-level=3" },
		c_flags = { "-O3" },
		cpp_flags = { "-O3" },
	},
	size = {
		level = "s",
		description = "Optimize for size",
		rust_flags = { "-C", "opt-level=s" },
		c_flags = { "-Os" },
		cpp_flags = { "-Os" },
	},
	size_aggressive = {
		level = "z",
		description = "Aggressively optimize for size",
		rust_flags = { "-C", "opt-level=z" },
		c_flags = { "-Oz" }, -- Clang only
		cpp_flags = { "-Oz" }, -- Clang only
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

function M.get_build_profile(profile_name)
	return M.build_profiles[profile_name] or M.build_profiles.debug
end

function M.get_optimization_flags(language, level)
	local opt_config = M.optimization_levels[level]
	if not opt_config then
		error(("Unknown optimization level: %s"):format(level))
	end

	local flag_key = language .. "_flags"
	return opt_config[flag_key] or {}
end

function M.get_debug_flags(language, level)
	local debug_config = M.debug_levels[level]
	if not debug_config then
		error(("Unknown debug level: %s"):format(level))
	end

	local flag_key = language .. "_flags"
	return debug_config[flag_key] or {}
end

function M.resolve_build_config(target_config, language)
	local profile_name = target_config.profile or "debug"
	if target_config.opt_level then
		if target_config.opt_level == "0" then
			profile_name = "debug"
		elseif target_config.opt_level == "2" or target_config.opt_level == "3" then
			profile_name = "release"
		elseif target_config.opt_level == "s" or target_config.opt_level == "z" then
			profile_name = "size"
		end
	end

	local profile = M.get_build_profile(profile_name)

	local build_config = {
		profile = profile_name,
		optimization = profile.optimization,
		debug_info = profile.debug_info,
		defines = profile.defines or {},
		is_release = profile_name ~= "debug" and profile_name ~= "dev",
	}

	if target_config.optimization then
		build_config.optimization = target_config.optimization
	elseif target_config.opt_level then
		local opt_level = target_config.opt_level
		if opt_level == "0" or opt_level == 0 then
			build_config.optimization = "debug"
		elseif opt_level == "1" or opt_level == 1 then
			build_config.optimization = "basic"
		elseif opt_level == "2" or opt_level == 2 then
			build_config.optimization = "some"
		elseif opt_level == "3" or opt_level == 3 then
			build_config.optimization = "full"
		elseif opt_level == "s" then
			build_config.optimization = "size"
		elseif opt_level == "z" then
			build_config.optimization = "size_aggressive"
		end
	end
	if target_config.debug_info then
		local debug_info = target_config.debug_info
		if debug_info == "0" or debug_info == 0 then
			build_config.debug_info = "none"
		elseif debug_info == "1" or debug_info == 1 then
			build_config.debug_info = "lines"
		elseif debug_info == "2" or debug_info == 2 then
			build_config.debug_info = "full"
		else
			build_config.debug_info = debug_info
		end
	end
	if target_config.defines then
		for k, v in pairs(target_config.defines) do
			build_config.defines[k] = v
		end
	end

	build_config.opt_flags = M.get_optimization_flags(language, build_config.optimization)
	build_config.debug_flags = M.get_debug_flags(language, build_config.debug_info)

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
	header = { "src/**/*.h", "src/**/*.hpp", "**/*.h", "**/*.hpp" },
}

function M.get_source_patterns(language)
	return M.source_patterns[language] or {}
end

local requested_components_by_target = nil

function M.should_target_be_built(target_name)
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

function M.should_build_component(component_name, target_name, dependencies)
	if forge.config.target_filters and #forge.config.target_filters > 0 then
		local should_build = false
		for _, filter in ipairs(forge.config.target_filters) do
			if filter == target_name then
				should_build = true
				break
			end
		end
		if not should_build then
			return false
		end
	end

	if not forge.config.component_filters or #forge.config.component_filters == 0 then
		return true
	end

	if not requested_components_by_target then
		requested_components_by_target = {}
	end

	if not requested_components_by_target[target_name] then
		requested_components_by_target[target_name] = {}
		for _, filter in ipairs(forge.config.component_filters) do
			requested_components_by_target[target_name][filter] = true
		end
	end

	local requested_for_target = requested_components_by_target[target_name]

	if requested_for_target[component_name] then
		if dependencies then
			for dep_name, _ in pairs(dependencies) do
				if not requested_for_target[dep_name] then
					requested_for_target[dep_name] = true
					forge.log.debug(
						("Component '%s' depends on '%s' for target '%s', adding to build set"):format(
							component_name,
							dep_name,
							target_name
						)
					)
				end
			end
		end
		return true
	end

	return false
end

return M
