local M = {}

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

function M.resolve_sources(sources, base_path)
	local resolved = {}
	base_path = base_path or forge.project.root

	for _, src in ipairs(sources) do
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

function M.get_target_triple_string(target)
	if target.canonical_name then
		return target.canonical_name
	end
	return ("%s-%s-%s-%s"):format(target.arch, target.vendor or "unknown", target.os, target.abi)
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
	local host_target = M.get_host_target()
	local gcc_cmd = is_cpp and "g++" or "gcc"

	if target.arch ~= host_target.arch or target.os ~= host_target.os then
		if target.arch == "aarch64" and target.os == "linux" then
			gcc_cmd = is_cpp and "aarch64-linux-gnu-g++" or "aarch64-linux-gnu-gcc"
		elseif target.arch == "arm" and target.os == "linux" then
			gcc_cmd = is_cpp and "arm-linux-gnueabihf-g++" or "arm-linux-gnueabihf-gcc"
		elseif target.arch == "x86_64" and target.os == "windows" then
			gcc_cmd = is_cpp and "x86_64-w64-mingw32-g++" or "x86_64-w64-mingw32-gcc"
		elseif target.arch == "i686" and target.os == "windows" then
			gcc_cmd = is_cpp and "i686-w64-mingw32-g++" or "i686-w64-mingw32-gcc"
		end
	end

	return gcc_cmd
end

function M.get_zig_target_string(target)
	local zig_target = target.arch

	if target.os == "windows" then
		zig_target = zig_target .. "-windows"
		if target.abi == "gnu" then
			zig_target = zig_target .. "-gnu"
		elseif target.abi == "msvc" then
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
		if target.abi and target.abi ~= "none" and target.abi ~= "macho" then
			zig_target = zig_target .. "-" .. target.abi
		end
	elseif target.os == "freebsd" then
		zig_target = zig_target .. "-freebsd"
	elseif target.os == "wasi" then
		zig_target = zig_target .. "-wasi"
	else
		zig_target = zig_target .. "-" .. target.os
		if target.abi then
			zig_target = zig_target .. "-" .. target.abi
		end
	end

	return zig_target
end

function M.is_native_target(target)
	local host_target = M.get_host_target()
	return target.arch == host_target.arch
		and target.os == host_target.os
		and (target.abi == host_target.abi or not target.abi)
end

return M
