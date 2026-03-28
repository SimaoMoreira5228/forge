local common = require("@prelude/js/js_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = target_common.canonical_targets

-- Helper to resolve toolchain components
local function resolve_pm_bin(pm, info)
	if pm == "pnpm" or pm == "yarn" then
		-- npm is usually the wrapper for these if they are global, 
		-- but here we assume pnpm/yarn might be on PATH or part of the toolchain info if we expand it.
		-- For now, we search in toolchain paths.
		return pm
	end
	return info[pm] or pm
end

function M.project(args)
	local name = args.name
	local targets = args.targets or { host = { target = forge.target:host() } }
	local src_dir = args.src_dir or "."
	local srcs = args.srcs or forge.source.glob(forge.path.join({ src_dir, "**/*" }))
	local build_script = args.build_script or "build"
	local output_dirs = args.outputs or { "dist" }
	local install = args.install ~= false

	for target, config in pairs(targets) do
		local info = common.resolve_runtime(config.runtime or "node")
		local pm = args.package_manager or common.detect_package_manager(src_dir)
		local env = common.get_js_env(info)
		if args.env then for k, v in pairs(args.env) do env[k] = v end end

		-- Ensure bin_dir is in PATH for shell scripts
		if info.bin_dir then
			env.PATH = info.bin_dir .. (forge.platform:os() == "windows" and ";" or ":") .. (env.PATH or "")
		end

		local pm_bin = resolve_pm_bin(pm, info) or "node"
		
		-- Use a shell wrapper to handle install + build if requested
		local command = "/bin/bash"
		if forge.platform:os() == "windows" then command = "cmd.exe" end

		local script = ""
		if install then
			if pm == "npm" then script = "npm install && "
			elseif pm == "pnpm" then script = "pnpm install --frozen-lockfile && "
			elseif pm == "yarn" then script = "yarn install --frozen-lockfile && "
			elseif pm == "bun" then script = "bun install && "
			end
		end

		if pm == "deno" then
			script = script .. "deno task " .. build_script
		else
			script = script .. pm_bin .. " run " .. build_script
		end

		local cargo_args = {}
		if forge.platform:os() == "windows" then
			cargo_args = { "/c", script }
		else
			cargo_args = { "-c", script }
		end

		local outputs = {}
		for _, out in ipairs(output_dirs) do
			table.insert(outputs, forge.path.join({ src_dir, out }))
		end

		forge.graph.custom {
			name = name,
			target = target,
			command = command,
			args = cargo_args,
			srcs = srcs,
			outputs = outputs,
			env = env,
			workdir = forge.project.root,
		}
	end
end

function M.binary(args)
	local name = args.name
	local targets = args.targets or { host = { target = forge.target:host() } }
	local main = args.main or "index.js"
	local srcs = args.srcs or { main }

	for target, config in pairs(targets) do
		local info = common.resolve_runtime(config.runtime or "node")
		local env = common.get_js_env(info)
		if args.env then for k, v in pairs(args.env) do env[k] = v end end

		local node_bin = info.node or "node"
		local cmd_args = { main }
		if args.args then
			for _, v in ipairs(args.args) do table.insert(cmd_args, v) end
		end

		forge.graph.custom {
			name = name,
			target = target,
			command = node_bin,
			args = cmd_args,
			srcs = srcs,
			outputs = { name }, -- Stub output
			env = env,
			workdir = forge.project.root,
		}
	end
end

function M.library(args)
	local name = args.name
	local src_dir = args.src_dir or "."
	local srcs = args.srcs or forge.source.glob(forge.path.join({ src_dir, "**/*.{js,ts,jsx,tsx}" }))
	local targets = args.targets or { host = { target = forge.target:host() } }
	
	-- For libraries, we often just want to run a build task that emits to a folder
	M.project(args)
end

function M.bundle(args)
	local name = args.name
	local entry = args.entry or "src/index.js"
	local output = args.output or "dist/bundle.js"
	local targets = args.targets or { host = { target = forge.target:host() } }
	local format = args.format or "esm"
	local pm = args.package_manager or common.detect_package_manager(".")

	for target, config in pairs(targets) do
		local info = common.resolve_runtime(config.runtime or "node")
		local env = common.get_js_env(info)
		
		-- Try to use esbuild if available, otherwise fallback to pm run build
		local build_cmd = ""
		if pm == "bun" then
			build_cmd = "bun build " .. entry .. " --outfile=" .. output .. " --format=" .. format
		else
			-- Assume esbuild might be in node_modules/.bin
			local esbuild = forge.path.join({ forge.project.root, "node_modules", ".bin", "esbuild" })
			if forge.fs.exists(esbuild) then
				build_cmd = esbuild .. " " .. entry .. " --bundle --outfile=" .. output .. " --format=" .. format
			else
				-- Fallback to a generic build script
				build_cmd = (resolve_pm_bin(pm, info) or pm) .. " run build"
			end
		end

		local command = "/bin/bash"
		if forge.platform:os() == "windows" then command = "cmd.exe" end
		local shell_args = forge.platform:os() == "windows" and { "/c", build_cmd } or { "-c", build_cmd }

		forge.graph.custom {
			name = name,
			target = target,
			command = command,
			args = shell_args,
			srcs = { entry },
			outputs = { output },
			env = env,
			workdir = forge.project.root,
		}
	end
end

-- TypeScript specific rules
M.ts = {}
function M.ts.library(args)
	local name = args.name
	local tsconfig = args.tsconfig or "tsconfig.json"
	local out_dir = args.out_dir or "dist"
	local targets = args.targets or { host = { target = forge.target:host() } }

	for target, config in pairs(targets) do
		local info = common.resolve_runtime(config.runtime or "node")
		local env = common.get_js_env(info)
		
		-- Use tsc from node_modules or toolchain if available
		local tsc = forge.path.join({ forge.project.root, "node_modules", ".bin", "tsc" })
		if not forge.fs.exists(tsc) then
			-- Fallback to npx tsc
			tsc = "npx tsc"
		end

		local command = "/bin/bash"
		if forge.platform:os() == "windows" then command = "cmd.exe" end
		local build_cmd = tsc .. " -p " .. tsconfig .. " --outDir " .. out_dir
		local shell_args = forge.platform:os() == "windows" and { "/c", build_cmd } or { "-c", build_cmd }

		forge.graph.custom {
			name = name,
			target = target,
			command = command,
			args = shell_args,
			srcs = forge.source.glob("**/*.ts"),
			outputs = { out_dir },
			env = env,
			workdir = forge.project.root,
		}
	end
end

return M
