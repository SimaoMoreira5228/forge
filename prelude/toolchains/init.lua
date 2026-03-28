local common = require("@prelude/toolchains/common.lua")

local registry = {
	gcc = require("@prelude/toolchains/gcc.lua"),
	clang = require("@prelude/toolchains/clang.lua"),
	rust = require("@prelude/toolchains/rust.lua"),
	zig = require("@prelude/toolchains/zig.lua"),
	cmake = require("@prelude/toolchains/cmake.lua"),
	make = require("@prelude/toolchains/make.lua"),
	gdc = require("@prelude/toolchains/gdc.lua"),
	dmd = require("@prelude/toolchains/dmd.lua"),
	ldc = require("@prelude/toolchains/ldc.lua"),
	msvc = require("@prelude/toolchains/msvc.lua"),
	node = require("@prelude/toolchains/node.lua"),
	bun = require("@prelude/toolchains/bun.lua"),
	deno = require("@prelude/toolchains/deno.lua"),
}

local M = {}

local function module_for(name)
	return registry[name]
end

function M.resolve(name, options)
	local mod = module_for(name)
	if mod and mod.resolve then
		return mod.resolve(name, options)
	end
	return common.resolve(name, options)
end

function M.resolve_from_config(name, options)
	return common.resolve_from_config(name, options)
end

function M.sync(name, options)
	local mod = module_for(name)
	if mod and mod.sync then
		return mod.sync(name, options)
	end
	return common.sync(name, options)
end

function M.list_configured()
	return common.list_configured()
end

function M.sync_all()
	local configured = (forge.config and forge.config.toolchain) or {}
	local out = {}
	for name, _ in pairs(configured) do
		out[#out + 1] = M.sync(name)
	end
	table.sort(out, function(a, b)
		return a.name < b.name
	end)
	return out
end

function M.resolve_compiler(name, options)
	local mod = module_for(name)
	if mod and mod.resolve_compiler then
		return mod.resolve_compiler(name, options)
	end
	local info = M.sync(name, options)
	if not info.bin_dir then
		return nil
	end
	return { bin_dir = info.bin_dir }
end

return M
