local common = require("@prelude/toolchains/common.lua")

local M = {}

local function collect_paths(...)
	local out = {}
	for i = 1, select("#", ...) do
		local value = select(i, ...)
		if value then
			out[#out + 1] = value
		end
	end
	return out
end

local function first_existing(paths)
	for _, path in ipairs(paths) do
		if forge.fs.exists(path) then
			return path
		end
	end
	return nil
end

local function find_configure_dir(root)
	local direct = common.path_join({ root, "configure" })
	if forge.fs.exists(direct) then
		return root
	end

	for _, candidate in ipairs(forge.fs.walk(root, { recursive = true })) do
		if forge.path.basename(candidate) == "configure" then
			local parent = forge.path.dirname(candidate)
			if parent and forge.fs.is_dir(parent) then
				return parent
			end
		end
	end

	return nil
end

local function bootstrap_make_from_source(spec, info)
	local source_dir = find_configure_dir(info.path)
	if not source_dir then
		error(("Failed to locate GNU Make source in '%s'"):format(info.path))
	end

	local prefix = common.path_join({ spec.install_dir, ".install" })
	common.ensure_dir(prefix)

	local configure = forge.exec.run({
		command = "sh",
		args = { "configure", "--prefix", prefix },
		working_dir = source_dir,
	})
	if not configure.success then
		error(("Failed to configure GNU Make source: %s"):format(configure.stderr or "unknown error"))
	end

	local build = forge.exec.run({
		command = "make",
		args = { "-j4" },
		working_dir = source_dir,
	})
	if not build.success then
		build = forge.exec.run({
			command = "gmake",
			args = { "-j4" },
			working_dir = source_dir,
		})
	end
	if not build.success then
		error(("Failed to build GNU Make source: %s"):format(build.stderr or "unknown error"))
	end

	local install = forge.exec.run({
		command = "make",
		args = { "install" },
		working_dir = source_dir,
	})
	if not install.success then
		install = forge.exec.run({
			command = "gmake",
			args = { "install" },
			working_dir = source_dir,
		})
	end
	if not install.success then
		error(("Failed to install GNU Make source: %s"):format(install.stderr or "unknown error"))
	end

	local bin_dir = common.path_join({ prefix, "bin" })
	local make_bin = first_existing({
		common.path_join({ bin_dir, "make" }),
		common.path_join({ bin_dir, "gmake" }),
	})

	if not make_bin then
		error(("GNU Make install completed but binary not found in '%s'"):format(bin_dir))
	end

	return {
		bin_dir = bin_dir,
		make = make_bin,
	}
end

function M.resolve(name, options)
	return common.resolve(name, options)
end

function M.sync(name, options)
	return common.sync(name, options, {
		after_sync = function(spec, info)
			local existing = first_existing(
				collect_paths(
					info.make,
					info.bin_dir and common.path_join({ info.bin_dir, "make" }) or nil,
					info.bin_dir and common.path_join({ info.bin_dir, "gmake" }) or nil
				)
			)
			if existing then
				return { make = existing }
			end

			if spec.from == "version" or spec.from == "url" then
				return bootstrap_make_from_source(spec, info)
			end

			return nil
		end,
	})
end

function M.resolve_compiler(name, options)
	local info = M.sync(name, options)
	if not info then
		return nil
	end

	local make_bin = first_existing(
		collect_paths(
			info.make,
			info.bin_dir and common.path_join({ info.bin_dir, "make" }) or nil,
			info.bin_dir and common.path_join({ info.bin_dir, "gmake" }) or nil
		)
	)

	return {
		make = make_bin,
		bin_dir = info.bin_dir,
	}
end

return M
