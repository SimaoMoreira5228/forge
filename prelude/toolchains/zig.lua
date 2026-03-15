local common = require("@prelude/toolchains/common.lua")

local M = {}

local function resolve_zig_binary(info)
	if info.bin_dir then
		local in_bin = common.path_join({ info.bin_dir, "zig" })
		if forge.fs.exists(in_bin) then
			return in_bin, info.bin_dir
		end
	end

	if info.path then
		local direct = common.path_join({ info.path, "zig" })
		if forge.fs.exists(direct) then
			return direct, info.path
		end

		for _, candidate in ipairs(forge.fs.walk(info.path, { recursive = true })) do
			if
				forge.path.basename(candidate) == "zig"
				and forge.fs.exists(candidate)
				and not forge.fs.is_dir(candidate)
				and not tostring(candidate):find("/lib/")
			then
				return candidate, forge.path.dirname(candidate)
			end
		end
	end

	return nil, info.bin_dir
end

function M.resolve(name, options)
	return common.resolve(name, options)
end

function M.sync(name, options)
	return common.sync(name, options, {
		after_sync = function(_, info)
			local zig, bin_dir = resolve_zig_binary(info)
			if zig then
				return {
					zig = zig,
					bin_dir = bin_dir,
				}
			end
			return nil
		end,
	})
end

function M.resolve_compiler(name, options)
	local info = M.sync(name, options)
	local zig = info and info.zig or nil
	local bin_dir = info and info.bin_dir or nil
	if not zig then
		zig, bin_dir = resolve_zig_binary(info)
	end
	if not zig then
		return nil
	end

	return {
		zig = zig,
		bin_dir = bin_dir,
	}
end

return M
