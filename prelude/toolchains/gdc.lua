local common = require("@prelude/toolchains/common.lua")

local M = {}

local function ensure_prefix_links(bin_dir)
	local gdc_variants = forge.fs.glob(common.path_join({ bin_dir, "*-gdc" }))
	if #gdc_variants == 0 then
		return nil
	end

	local gdc_path = gdc_variants[1]
	local gdc_name = forge.path.basename(gdc_path)
	local prefix = gdc_name:sub(1, #gdc_name - 3)

	local map = {
		{ from = prefix .. "gdc", to = "gdc" },
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

	local gdc = common.path_join({ info.bin_dir, "gdc" })
	return {
		d = forge.fs.exists(gdc) and gdc or nil,
		bin_dir = info.bin_dir,
	}
end

return M
