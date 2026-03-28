local common = require("@prelude/toolchains/common.lua")

local M = {}

function M.resolve(name, options)
	return common.resolve(name, options)
end

function M.sync(name, options)
	return common.sync(name, options, {
		after_sync = function(spec, info)
			local base_dir = info.path
			if not base_dir or not forge.fs.exists(base_dir) then return end

			local rustc_rustlib = common.path_join({ base_dir, "rustc", "lib", "rustlib" })
			if not forge.fs.exists(rustc_rustlib) then return end

			for _, entry in ipairs(forge.fs.walk(base_dir, { recursive = false })) do
				if forge.fs.is_dir(entry) then
					local comp_name = forge.path.basename(entry)
					if comp_name ~= "rustc" and comp_name ~= "cargo" then
						local comp_rustlib = common.path_join({ entry, "lib", "rustlib" })
						if forge.fs.exists(comp_rustlib) then
							print("[Forge] Merging rust component: " .. comp_name)
							os.execute(string.format("cp -rn '%s'/* '%s'/", comp_rustlib, rustc_rustlib))
						end
					end
				end
			end
		end
	})
end

function M.resolve_compiler(name, options)
	local info = M.sync(name, options)
	if not info.bin_dir then
		return nil
	end

	local rustc = common.path_join({ info.bin_dir, "rustc" })
	local cargo = common.path_join({ info.bin_dir, "cargo" })
	
	local paths = {}
	if info.path and forge.fs.exists(info.path) then
		for _, entry in ipairs(forge.fs.walk(info.path, { recursive = true })) do
			if forge.fs.is_dir(entry) and forge.path.basename(entry) == "bin" then
				table.insert(paths, entry)
			end
		end
	end

	return {
		rustc = forge.fs.exists(rustc) and rustc or nil,
		cargo = forge.fs.exists(cargo) and cargo or nil,
		bin_dir = info.bin_dir,
		paths = paths,
	}
end

return M
