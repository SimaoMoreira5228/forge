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

	local node_name = forge.platform:os() == "windows" and "node.exe" or "node"
	local npm_name = forge.platform:os() == "windows" and "npm.cmd" or "npm"
	local npx_name = forge.platform:os() == "windows" and "npx.cmd" or "npx"

	local node = common.path_join({ info.bin_dir, node_name })
	local npm = common.path_join({ info.bin_dir, npm_name })
	local npx = common.path_join({ info.bin_dir, npx_name })

	if not forge.fs.exists(node) then
		node = common.path_join({ info.path, node_name })
		npm = common.path_join({ info.path, npm_name })
		npx = common.path_join({ info.path, npx_name })
	end

	local paths = { info.bin_dir }
	if info.path and info.path ~= info.bin_dir then
		table.insert(paths, info.path)
	end

	return {
		node = forge.fs.exists(node) and node or nil,
		npm = forge.fs.exists(npm) and npm or nil,
		npx = forge.fs.exists(npx) and npx or nil,
		bin_dir = info.bin_dir,
		paths = paths,
	}
end

return M
