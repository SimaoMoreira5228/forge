local compiler_common = require("@prelude/compiler_common.lua")

local M = {}

local targets_list = forge.target:list()
local predefined = {}
for _, t in ipairs(targets_list) do
	predefined[t.name] = t
end
M.predefined_targets = predefined

M.compilers = {
	gcc = "g++",
	clang = "clang++",
	zig = "zig c++",
	msvc = "cl",
}


M.standards = {
	cpp98 = "c++98",
	cpp03 = "c++03",
	cpp11 = "c++11",
	cpp14 = "c++14",
	cpp17 = "c++17",
	cpp20 = "c++20",
	cpp23 = "c++23",
	cpp26 = "c++26",
}

M.get_host_target = compiler_common.get_host_target
M.get_target_triple_string = compiler_common.get_target_triple_string
M.resolve_sources = compiler_common.resolve_sources
M.resolve_includes = compiler_common.resolve_includes

function M.get_compiler_for_target(compiler, target, standard, compiler_path)
	local is_msvc = (compiler == "msvc")
	local std_flag = nil
	if standard then
		if is_msvc then
			std_flag = "/std:" .. standard
		else
			std_flag = "-std=" .. standard
		end
	end
	
	local args = {}
	local profile = forge.profile
	if profile and profile:name() == "coverage" then
		if compiler == "clang" or compiler == "zig" then
			table.insert(args, "-fprofile-instr-generate")
			table.insert(args, "-fcoverage-mapping")
		elseif compiler == "gcc" then
			table.insert(args, "--coverage")
		end
	end

	if compiler_path then
		if std_flag then
			table.insert(args, std_flag)
		end
		return { id = compiler, command = compiler_path, args = args }
	end

	if compiler == "zig" then
		local zig_target = compiler_common.get_zig_target_string(target)
		table.insert(args, "c++")
		table.insert(args, "-target")
		table.insert(args, zig_target)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { id = "zig", command = "zig", args = args }
	elseif compiler == "clang" then
		local configured_clang = compiler_common.get_configured_compiler("clang", true, target)
		forge.log.warn("get_configured_compiler(clang) returned: " .. tostring(configured_clang))
		if configured_clang then
			if std_flag then
				table.insert(args, std_flag)
			end
			return { id = "clang", command = configured_clang, args = args }
		end

		local clang_target = M.get_target_triple_string(target)
		table.insert(args, "--target=" .. clang_target)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { id = "clang", command = "clang++", args = args }
	elseif compiler == "msvc" then
		local configured_msvc = compiler_common.get_configured_compiler("msvc", true, target)
		if configured_msvc then
			if std_flag then
				table.insert(args, std_flag)
			end
			return { id = "msvc", command = configured_msvc, args = args, env = configured_msvc.env }
		end

		if std_flag then
			table.insert(args, std_flag)
		end
		return { id = "msvc", command = "cl.exe", args = args }
	else
		local info = compiler_common.get_gcc_info(target, true)
		if std_flag then
			table.insert(info.args, std_flag)
		end
		info.id = "gcc"
		return info
	end
end


function M.validate_standard(standard)
	if not standard then
		return nil
	end

	if M.standards[standard] then
		return M.standards[standard]
	end

	if forge.string.starts_with(standard, "c++") then
		return standard
	end

	forge.log.warn(("Unknown C++ standard: %s, using default"):format(standard))
	return nil
end

local function resolve_clangxx_command(target)
	local configured_clang = compiler_common.get_configured_compiler("clang", true, target)
	if configured_clang then
		return configured_clang
	end
	return "clang++"
end

function M.get_clang_include_args(target)
	local clangxx = resolve_clangxx_command(target)
	local result = forge.exec.run({
		command = clangxx,
		args = { "-v", "-E", "-x", "c++", "/dev/null" },
	})
	local output = result.stdout

	local includes = {}
	for path in output:gmatch("[^\n]+") do
		local inc = path:match("^%s*(/usr.-)%s*$")
		if inc and inc:find("include") then
			table.insert(includes, "-I" .. inc)
		end
	end
	return includes
end

function M.scan_modules_with_msvc(sources, include_dirs, target)
	if #sources == 0 then
		return { modules = {}, deps = {}, provides = {} }
	end

	local project_path = forge.project.root
	local msvc = M.get_compiler_for_target("msvc", target)
	if not msvc or not msvc.command then
		error("MSVC compiler not found for scanning")
	end

	local provides = {}
	local requires = {}

	for _, src in ipairs(sources) do
		local abs_src = src
		if not src:match("^/") and not src:match("^%a:") then
			abs_src = project_path .. "/" .. src
		end

		local scan_json = project_path .. "/forge-out/msvc-scan.json"
		local scan_args = { "/nologo", "/scanDependencies", scan_json, "/c", abs_src, "/std:c++20" }
		for _, inc in ipairs(include_dirs) do
			local abs_inc = inc
			if not inc:match("^/") and not inc:match("^%a:") then
				abs_inc = project_path .. "/" .. inc
			end
			table.insert(scan_args, "/I" .. abs_inc)
		end

		local result = forge.exec.run({
			command = msvc.command,
			args = scan_args,
			env = msvc.env,
		})

		if result.exit_code == 0 and forge.fs.exists(scan_json) then
			local content = forge.fs.read(scan_json)
			local data = forge.json.decode(content)
			
			if data and data.Data then
				for _, entry in ipairs(data.Data.ProvidedModules or {}) do
					provides[entry.Name] = {
						name = entry.Name,
						is_interface = true,
						source_file = abs_src,
					}
				end
				for _, entry in ipairs(data.Data.ImportedModules or {}) do
					if not requires[abs_src] then requires[abs_src] = {} end
					table.insert(requires[abs_src], entry.Name)
				end
			end
			forge.fs.remove(scan_json)
		else
			forge.log.warn("MSVC scan failed for " .. abs_src .. ": " .. (result.stderr or "unknown error"))
		end
	end

	return {
		modules = provides,
		deps = requires,
		provides = provides,
		requires = requires,
		cycles = {},
	}
end

function M.scan_modules_with_clang_scan_deps(sources, include_dirs, target)
	if #sources == 0 then
		return { modules = {}, deps = {}, provides = {} }
	end

	local project_path = forge.project.root
	local effective_target = target or M.get_host_target()
	local target_str = M.get_target_triple_string(effective_target)
	local std = "-std=c++20"
	local clangxx = resolve_clangxx_command(effective_target)

	local include_args = M.get_clang_include_args(effective_target)

	local compile_commands = {}
	for _, src in ipairs(sources) do
		local abs_src = src
		if not src:match("^/") then
			abs_src = project_path .. "/" .. src
		end

		local cmd = { clangxx, "--target=" .. target_str, std }
		for _, inc in ipairs(include_args) do
			table.insert(cmd, inc)
		end
		for _, inc in ipairs(include_dirs) do
			local abs_inc = inc
			if not inc:match("^/") then
				abs_inc = project_path .. "/" .. inc
			end
			table.insert(cmd, "-I" .. abs_inc)
		end
		table.insert(cmd, "-c")
		table.insert(cmd, abs_src)

		table.insert(compile_commands, {
			directory = project_path,
			command = table.concat(cmd, " "),
			file = abs_src,
		})
	end

	local json_path = project_path .. "/forge-out/clang-scan-deps.json"
	local tmp_compile_commands = project_path .. "/forge-out/compile_commands.json"

	forge.fs.write(tmp_compile_commands, forge.json.encode(compile_commands))

	local clang_scan_result = forge.exec.run({
		command = "clang-scan-deps",
		args = { "-format=p1689", "-compilation-database=" .. tmp_compile_commands },
	})
	local output = clang_scan_result.stdout

	forge.fs.remove(tmp_compile_commands)

	if not output:match("^%s*{") then
		error("clang-scan-deps failed: " .. output)
	end

	local result = forge.json.decode(output)
	if not result or not result.rules then
		error("Failed to parse clang-scan-deps output")
	end

	local provides = {}
	local requires = {}

	for _, rule in ipairs(result.rules) do
		if rule.provides then
			for _, p in ipairs(rule.provides) do
				provides[p["logical-name"]] = {
					name = p["logical-name"],
					is_interface = p["is-interface"] or false,
					source_file = p["source-path"],
				}
			end
		end
		if rule.requires then
			for _, r in ipairs(rule.requires) do
				local src_file = nil
				for _, cmd in ipairs(compile_commands) do
					src_file = cmd.file
					break
				end
				if not requires[src_file] then
					requires[src_file] = {}
				end
				table.insert(requires[src_file], r["logical-name"])
			end
		end
	end

	return {
		modules = provides,
		deps = requires,
		provides = provides,
		requires = requires,
		cycles = {},
	}
end

function M.scan_modules(sources, include_dirs, target, compiler_name)
	if type(target) == "string" then
		target = forge.target.resolve(target)
	end
	target = target or M.get_host_target()
	compiler_name = compiler_name or (forge.config and forge.config.toolchain and forge.config.toolchain.cpp and forge.config.toolchain.cpp.compiler) or "gcc"
	
	if target.os == "windows" and compiler_name == "msvc" then
		return M.scan_modules_with_msvc(sources, include_dirs, target)
	end
	
	local result = M.scan_modules_with_clang_scan_deps(sources, include_dirs, target)

	local module_count = 0
	for _ in pairs(result.modules) do
		module_count = module_count + 1
	end
	forge.log.info(("clang-scan-deps found %d modules"):format(module_count))

	return result
end

function M.has_modules(sources)
	for _, source in ipairs(sources) do
		local ok, content = pcall(forge.fs.read, source)
		if ok and content:match("export%s+module") then
			return true
		end
	end
	return false
end

function M.get_module_source_files(sources)
	local module_sources = {}
	for _, source in ipairs(sources) do
		local ok, content = pcall(forge.fs.read, source)
		if ok and content:match("export%s+module") then
			table.insert(module_sources, source)
		end
	end
	return module_sources
end

function M.filter_out_module_sources(sources)
	local filtered = {}
	for _, source in ipairs(sources) do
		local is_module = false
		local ok, content = pcall(forge.fs.read, source)
		if ok and content:match("export%s+module") then
			is_module = true
		end
		if not is_module then
			table.insert(filtered, source)
		end
	end
	return filtered
end

return M
