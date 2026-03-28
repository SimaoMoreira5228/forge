local common = require("@prelude/toolchains/common.lua")

local M = {}

function M.resolve(name, options)
	return common.resolve(name, options)
end

function M.sync(name, options)
	return common.sync(name, options)
end

function M.resolve_compiler(name, options)
	local info = M.sync(name, options)
	local bun_name = forge.platform:os() == "windows" and "bun.exe" or "bun"
	local bun = common.find_binary_path(info.path, bun_name)

	if not bun then
		return nil
	end

	local bin_dir = forge.path.dirname(bun)
	local paths = { bin_dir }
	if info.path and info.path ~= bin_dir then
		table.insert(paths, info.path)
	end

	return {
		bun = bun,
		bin_dir = bin_dir,
		paths = paths,
	}
end

return M
