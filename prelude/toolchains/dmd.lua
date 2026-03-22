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

	local candidates = {
		common.path_join({ info.bin_dir, "dmd" }),
		common.path_join({ info.bin_dir, "linux/bin64/dmd" }),
		common.path_join({ info.bin_dir, "linux/bin32/dmd" }),
		common.path_join({ info.bin_dir, "osx/bin/dmd" }),
		common.path_join({ info.bin_dir, "windows/bin64/dmd.exe" }),
	}

	local dmd_path
	for _, cand in ipairs(candidates) do
		if forge.fs.exists(cand) then
			dmd_path = cand
			break
		end
	end

	return {
		d = dmd_path,
		bin_dir = info.bin_dir,
	}
end

return M
