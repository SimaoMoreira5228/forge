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

	local cmake = common.path_join({ info.bin_dir, "cmake" })
	local ctest = common.path_join({ info.bin_dir, "ctest" })
	return {
		cmake = forge.fs.exists(cmake) and cmake or nil,
		ctest = forge.fs.exists(ctest) and ctest or nil,
		bin_dir = info.bin_dir,
	}
end

return M
