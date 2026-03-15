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
	if not info.bin_dir then
		return nil
	end

	local rustc = common.path_join({ info.bin_dir, "rustc" })
	local cargo = common.path_join({ info.bin_dir, "cargo" })
	return {
		rustc = forge.fs.exists(rustc) and rustc or nil,
		cargo = forge.fs.exists(cargo) and cargo or nil,
		bin_dir = info.bin_dir,
	}
end

return M
