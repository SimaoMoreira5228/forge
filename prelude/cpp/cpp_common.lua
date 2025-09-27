local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = {}
for name, canonical_info in pairs(target_common.canonical_targets) do
	M.predefined_targets[name] = {
		arch = canonical_info.arch,
		os = canonical_info.os,
		abi = canonical_info.abi,
		vendor = canonical_info.vendor,
		canonical_name = canonical_info.canonical_name,
	}
end

M.compilers = {
	gcc = "g++",
	clang = "clang++",
	zig = "zig c++",
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

function M.get_host_target()
	local arch_map = {
		x86_64 = "x86_64",
		x86 = "i686",
		i386 = "i686",
		aarch64 = "aarch64",
		arm64 = "aarch64",
		arm = "arm",
		armv7 = "arm",
	}

	local os_map = {
		linux = "linux",
		macos = "macos",
		darwin = "macos",
		windows = "windows",
		win32 = "windows",
		freebsd = "freebsd",
		openbsd = "openbsd",
		netbsd = "netbsd",
	}

	local arch = arch_map[forge.platform.arch()] or "x86_64"
	local os_name = os_map[forge.platform.os()] or "linux"

	local abi = "gnu"
	if os_name == "windows" then
		abi = "msvc"
	elseif os_name == "macos" then
		abi = "macho"
	end

	return { arch = arch, os = os_name, abi = abi }
end

function M.get_target_triple_string(target)
	if target.canonical_name then
		return target.canonical_name
	end
	return ("%s-%s-%s-%s"):format(target.arch, target.vendor or "unknown", target.os, target.abi)
end

function M.get_compiler_for_target(compiler, target, standard)
	local std_flag = standard and ("-std=" .. standard) or nil
	local args = {}

	if compiler == "zig" then
		local zig_target = target.arch
		if target.os == "windows" then
			zig_target = zig_target .. "-windows-gnu"
		elseif target.os == "linux" then
			zig_target = zig_target .. "-linux"
			if target.abi == "musl" then
				zig_target = zig_target .. "-musl"
			else
				zig_target = zig_target .. "-gnu"
			end
		elseif target.os == "macos" then
			zig_target = zig_target .. "-macos"
		elseif target.os == "freebsd" then
			zig_target = zig_target .. "-freebsd"
		elseif target.os == "wasi" then
			zig_target = zig_target .. "-wasi"
		end

		table.insert(args, "c++")
		table.insert(args, "-target")
		table.insert(args, zig_target)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = "zig", args = args }
	elseif compiler == "clang" then
		local clang_target = M.get_target_triple_string(target)
		table.insert(args, "--target=" .. clang_target)
		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = "clang++", args = args }
	else
		local cpp_cmd = "g++"
		local host_target = M.get_host_target()

		if target.arch ~= host_target.arch or target.os ~= host_target.os then
			if target.arch == "aarch64" and target.os == "linux" then
				cpp_cmd = "aarch64-linux-gnu-g++"
			elseif target.arch == "arm" and target.os == "linux" then
				cpp_cmd = "arm-linux-gnueabihf-g++"
			elseif target.arch == "x86_64" and target.os == "windows" then
				cpp_cmd = "x86_64-w64-mingw32-g++"
			elseif target.arch == "i686" and target.os == "windows" then
				cpp_cmd = "i686-w64-mingw32-g++"
			end
		end

		if std_flag then
			table.insert(args, std_flag)
		end
		return { command = cpp_cmd, args = args }
	end
end

function M.resolve_sources(srcs, base_path)
	local resolved = {}
	base_path = base_path or forge.project.root

	for _, src in ipairs(srcs) do
		local full_path = forge.path.is_absolute(src) and src or forge.path.join({ base_path, src })

		if src:match("[*?]") then
			local matches = forge.fs.glob(full_path)
			for _, match in ipairs(matches) do
				table.insert(resolved, match)
			end
		else
			if forge.fs.exists(full_path) then
				table.insert(resolved, full_path)
			end
		end
	end

	return resolved
end

function M.resolve_includes(includes, base_path)
	local resolved = {}
	base_path = base_path or forge.project.root

	for _, include in ipairs(includes or {}) do
		local full_path = forge.path.is_absolute(include) and include or forge.path.join({ base_path, include })
		if forge.fs.exists(full_path) then
			table.insert(resolved, full_path)
		end
	end

	return resolved
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

return M
