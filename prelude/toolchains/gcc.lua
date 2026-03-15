local common = require("@prelude/toolchains/common.lua")

local M = {}

local function ensure_prefix_links(bin_dir)
	local gcc_variants = forge.fs.glob(common.path_join({ bin_dir, "*-gcc" }))
	if #gcc_variants == 0 then
		return nil
	end

	local gcc_path = gcc_variants[1]
	local gcc_name = forge.path.basename(gcc_path)
	local prefix = gcc_name:sub(1, #gcc_name - 3)

	local map = {
		{ from = prefix .. "gcc", to = "gcc" },
		{ from = prefix .. "g++", to = "g++" },
		{ from = prefix .. "ar", to = "ar" },
		{ from = prefix .. "ld", to = "ld" },
	}

	for _, item in ipairs(map) do
		local src = common.path_join({ bin_dir, item.from })
		local dst = common.path_join({ bin_dir, item.to })
		if forge.fs.exists(src) and not forge.fs.exists(dst) then
			forge.fs.symlink(src, dst)
		end
	end

	return prefix
end

function M.resolve(name, options)
	return common.resolve(name, options)
end

function M.sync(name, options)
	return common.sync(name, options, {
		after_sync = function(_, info)
			if info.bin_dir then
				return { prefix = ensure_prefix_links(info.bin_dir) }
			end
			return nil
		end,
	})
end

function M.resolve_compiler(name, options)
	local info = M.sync(name, options)
	if not info.bin_dir then
		return nil
	end

	local gcc = common.path_join({ info.bin_dir, "gcc" })
	local gpp = common.path_join({ info.bin_dir, "g++" })
	return {
		c = forge.fs.exists(gcc) and gcc or nil,
		cpp = forge.fs.exists(gpp) and gpp or nil,
		bin_dir = info.bin_dir,
	}
end

return M
