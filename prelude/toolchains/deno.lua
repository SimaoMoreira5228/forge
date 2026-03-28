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
	local deno_name = forge.platform:os() == "windows" and "deno.exe" or "deno"
	local deno = common.find_binary_path(info.path, deno_name)

	if not deno then
		return nil
	end

	local bin_dir = forge.path.dirname(deno)
	local paths = { bin_dir }
	if info.path and info.path ~= bin_dir then
		table.insert(paths, info.path)
	end

	return {
		deno = deno,
		bin_dir = bin_dir,
		paths = paths,
	}
end

return M
