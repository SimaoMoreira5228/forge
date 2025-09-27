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
	gcc = "gcc",
	clang = "clang",
	zig = "zig cc",
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

function M.get_target_directory(target_name, variant_name)
	return target_common.get_target_directory(target_name, variant_name)
end

function M.get_compiler_for_target(compiler, target)
	local args = {}

	if compiler == "zig" then
		local zig_target = target.arch
		if target.os == "windows" then
			zig_target = zig_target .. "-windows"
			if target.abi == "gnu" then
				zig_target = zig_target .. "-gnu"
			else
				zig_target = zig_target .. "-msvc"
			end
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

		return { command = "zig", args = { "cc", "-target", zig_target } }
	elseif compiler == "clang" then
		local clang_target = M.get_target_triple_string(target)
		return { command = "clang", args = { "--target=" .. clang_target } }
	else
		local gcc_cmd = "gcc"
		local host_target = M.get_host_target()

		if target.arch ~= host_target.arch or target.os ~= host_target.os then
			if target.arch == "aarch64" and target.os == "linux" then
				gcc_cmd = "aarch64-linux-gnu-gcc"
			elseif target.arch == "arm" and target.os == "linux" then
				gcc_cmd = "arm-linux-gnueabihf-gcc"
			elseif target.arch == "x86_64" and target.os == "windows" then
				gcc_cmd = "x86_64-w64-mingw32-gcc"
			elseif target.arch == "i686" and target.os == "windows" then
				gcc_cmd = "i686-w64-mingw32-gcc"
			end
		end
		return { command = gcc_cmd, args = {} }
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

return M
