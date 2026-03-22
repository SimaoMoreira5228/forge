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

	local ldc2 = common.path_join({ info.bin_dir, "ldc2" })
	if not forge.fs.exists(ldc2) then
		ldc2 = common.path_join({ info.bin_dir, "ldc2.exe" })
	end

	return {
		d = forge.fs.exists(ldc2) and ldc2 or nil,
		bin_dir = info.bin_dir,
	}
end

return M
