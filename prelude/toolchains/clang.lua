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

	local clang = common.path_join({ info.bin_dir, "clang" })
	local clangpp = common.path_join({ info.bin_dir, "clang++" })
	
	forge.log.warn("clang.lua info.bin_dir = " .. tostring(info.bin_dir))
	forge.log.warn("clang.lua clang = " .. tostring(clang) .. " exists = " .. tostring(forge.fs.exists(clang)))
	forge.log.warn("clang.lua clangpp = " .. tostring(clangpp) .. " exists = " .. tostring(forge.fs.exists(clangpp)))
	
	return {
		c = forge.fs.exists(clang) and clang or nil,
		cpp = forge.fs.exists(clangpp) and clangpp or nil,
		bin_dir = info.bin_dir,
	}
end

return M
