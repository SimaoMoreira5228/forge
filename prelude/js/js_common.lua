local toolchains = require("@prelude/toolchains/init.lua")

local M = {}

function M.resolve_runtime(name)
	local info = toolchains.resolve_compiler(name or "node")
	if not info then
		error("JavaScript toolchain '" .. (name or "node") .. "' not found. Check your FORGE_ROOT.")
	end
	return info
end

function M.get_js_env(info, target_dir)
	local env = {
		NODE_ENV = forge.profile:name() == "release" and "production" or "development",
		PATH = table.concat(info.paths, ":") .. ":/usr/bin:/bin",
	}
	return env
end

function M.detect_package_manager(source_dir)
	local abs_dir = source_dir
	if not forge.path.is_absolute(source_dir) then
		abs_dir = forge.path.join({ forge.project.root, source_dir })
	end

	if forge.fs.exists(forge.path.join({ abs_dir, "pnpm-lock.yaml" })) then
		return "pnpm"
	elseif forge.fs.exists(forge.path.join({ abs_dir, "package-lock.json" })) then
		return "npm"
	elseif forge.fs.exists(forge.path.join({ abs_dir, "yarn.lock" })) then
		return "yarn"
	elseif forge.fs.exists(forge.path.join({ abs_dir, "bun.lockb" })) or forge.fs.exists(forge.path.join({ abs_dir, "bun.lock" })) then
		return "bun"
	end
	return "pnpm"
end

return M
