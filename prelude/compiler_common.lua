local M = {}
local toolchain_core = require("@prelude/toolchains/init.lua")

local function target_to_key(target)
	local target_info = target
	if type(target_info) == "string" then
		target_info = forge.target.resolve(target_info)
	end

	if type(target_info) ~= "table" then
		return nil
	end

	local os_name = target_info.os
	if os_name == "macos" then
		os_name = "darwin"
	end

	if not os_name or not target_info.arch then
		return nil
	end

	return os_name .. "-" .. target_info.arch
end

local function resolve_configured_compiler(toolchain_name, is_cpp, target)
	local target_key = target_to_key(target)
	local ok, resolved = pcall(toolchain_core.resolve_compiler, toolchain_name, { target_key = target_key })
	if not ok then
		forge.log.error("resolve_compiler pcall failed: " .. tostring(resolved))
		return nil
	end
	if not resolved then
		forge.log.error("resolve_compiler returned nil for " .. toolchain_name)
		return nil
	end

	local candidate = is_cpp and resolved.cpp or resolved.c
	if not candidate then
		forge.log.error("candidate was nil. c=" .. tostring(resolved.c) .. " cpp=" .. tostring(resolved.cpp))
		return nil
	end
	
	if forge.fs.exists(candidate) then
		return candidate
	else
		forge.log.error("candidate does not exist on disk: " .. candidate)
		return nil
	end
end

function M.get_configured_compiler(toolchain_name, is_cpp, target)
	return resolve_configured_compiler(toolchain_name, is_cpp, target)
end

function M.get_configured_tool_binary(toolchain_name, executable, target)
	local target_key = target_to_key(target)
	local ok, info = pcall(toolchain_core.sync, toolchain_name, { target_key = target_key })
	if not ok or not info then
		return nil
	end

	local direct = info[executable]
	if direct and forge.fs.exists(direct) then
		return direct
	end

	if info.bin_dir then
		local candidate = forge.path.join({ info.bin_dir, executable })
		if forge.fs.exists(candidate) then
			return candidate
		end
	end

	return nil
end

function M.get_host_target()
	return forge.target:host()
end

function M.resolve_sources(sources, base_path)
	local resolved = forge.source.resolve({ patterns = sources })
	return resolved
end

function M.resolve_includes(includes, base_path)
	local resolved = forge.source.includes({ dirs = includes or {} })
	return resolved
end

function M.get_target_triple_string(target)
	if type(target) == "table" and target.triple then
		return target.triple
	end
	if type(target) == "string" then
		local resolved = forge.target.resolve(target)
		if resolved and resolved.triple then
			return resolved.triple
		end
	end
	return target.canonical_name or tostring(target)
end

function M.to_absolute_path(path, base_path)
	if forge.path.is_absolute(path) then
		return path
	end
	return forge.path.join({ base_path or forge.project.root, path })
end

function M.ensure_dir(path)
	if not forge.fs.exists(path) then
		forge.fs.mkdir(path)
	end
end

function M.get_gcc_cross_compiler(target, is_cpp)
	local configured = resolve_configured_compiler("gcc", is_cpp, target)
	if configured then
		return configured
	end

	local host_target = forge.target:host()
	local gcc_cmd = is_cpp and "g++" or "gcc"

	local target_arch = type(target) == "table" and target.arch or (target.arch or "x86_64")
	local target_os = type(target) == "table" and target.os or (target.os or "linux")

	if target_arch ~= host_target.arch or target_os ~= host_target.os then
		if target_arch == "aarch64" and target_os == "linux" then
			gcc_cmd = is_cpp and "aarch64-linux-gnu-g++" or "aarch64-linux-gnu-gcc"
		elseif target_arch == "arm" and target_os == "linux" then
			gcc_cmd = is_cpp and "arm-linux-gnueabihf-g++" or "arm-linux-gnueabihf-gcc"
		elseif target_arch == "x86_64" and target_os == "windows" then
			gcc_cmd = is_cpp and "x86_64-w64-mingw32-g++" or "x86_64-w64-mingw32-gcc"
		elseif target_arch == "i686" and target_os == "windows" then
			gcc_cmd = is_cpp and "i686-w64-mingw32-g++" or "i686-w64-mingw32-gcc"
		end
	end

	return gcc_cmd
end

function M.get_zig_target_string(target)
	local target_info = type(target) == "table" and target or (forge.target.resolve(target) or {})
	local arch = target_info.arch or "x86_64"
	local os = target_info.os or "linux"
	local abi = target_info.abi or "gnu"

	local zig_target = arch

	if os == "windows" then
		zig_target = zig_target .. "-windows"
		if abi == "gnu" then
			zig_target = zig_target .. "-gnu"
		elseif abi == "msvc" then
			zig_target = zig_target .. "-msvc"
		end
	elseif os == "linux" then
		zig_target = zig_target .. "-linux"
		if abi == "musl" then
			zig_target = zig_target .. "-musl"
		else
			zig_target = zig_target .. "-gnu"
		end
	elseif os == "macos" then
		zig_target = zig_target .. "-macos"
	elseif os == "freebsd" then
		zig_target = zig_target .. "-freebsd"
	elseif os == "wasi" then
		zig_target = zig_target .. "-wasi"
	else
		zig_target = zig_target .. "-" .. os
		if abi and abi ~= "" then
			zig_target = zig_target .. "-" .. abi
		end
	end

	return zig_target
end

function M.is_native_target(target)
	local host_target = forge.target:host()
	local target_info = type(target) == "table" and target or (forge.target.resolve(target) or {})

	return target_info.arch == host_target.arch
		and target_info.os == host_target.os
		and (target_info.abi == host_target.abi or not target_info.abi)
end

return M
