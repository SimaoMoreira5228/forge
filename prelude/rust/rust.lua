local common = require("@prelude/rust/rust_common.lua")
local target_common = require("@prelude/target_common.lua")

local M = {}

M.predefined_targets = target_common.canonical_targets

local function normalize_deps(deps)
	if not deps then
		return {}
	end
	local normalized = {}
	for key, value in pairs(deps) do
		if type(key) == "number" then
			table.insert(normalized, value)
		else
			table.insert(normalized, key)
		end
	end
	return normalized
end

function M.binary(args)
	print("DEBUG: rust.binary called for " .. (args.name or "unknown"))
	local name = args.name
	local targets = args.targets or { host = { target = forge.target:host() } }
	local srcs = args.srcs or {}
	local deps = normalize_deps(args.deps or args.dependencies)

	for target, config in pairs(targets) do
		local profile = forge.profile:name() or "debug"
		local target_dir = common.get_target_dir(name, target, profile)
		local cargo = common.resolve_cargo(target)
		local env = common.get_cargo_env(target, target_dir)

		if args.env then
			for k, v in pairs(args.env) do env[k] = v end
		end

		local cargo_args = { "build", "--target-dir", target_dir }
		if profile == "release" then
			table.insert(cargo_args, "--release")
		end

		local output_path = forge.path.join({ target_dir, profile, name .. (forge.platform:os() == "windows" and ".exe" or "") })
		
		local forge_deps = {}
		local rustflags = ""
		for _, dep_name in ipairs(deps) do
			local outputs = forge.graph.get_component_outputs(dep_name, target)
			if #outputs > 0 then
				table.insert(forge_deps, dep_name)
				forge.log.info("Dependency '" .. dep_name .. "' has " .. #outputs .. " outputs")
				for _, output in ipairs(outputs) do
					forge.log.info("  Discovered output: " .. output)
					if output:match("%.a$") or output:match("%.lib$") then
						local dir = forge.path.dirname(output)
						local name = forge.path.basename(output):match("^lib(.+)%.a$") or forge.path.basename(output):match("^(.+)%.lib$")
						if name then
							forge.log.info("  Adding RUSTFLAGS: -L native=" .. dir .. " -l static=" .. name)
							rustflags = rustflags .. " -L native=" .. dir .. " -l static=" .. name
						end
					end
				end
			end
		end

		if rustflags ~= "" then
			env.RUSTFLAGS = (env.RUSTFLAGS or "") .. rustflags
		end

		if args.libs then
			local libs_flags = ""
			for _, lib in ipairs(args.libs) do
				libs_flags = libs_flags .. " -l " .. lib
			end
			env.RUSTFLAGS = (env.RUSTFLAGS or "") .. libs_flags
		end

		print("DEBUG: forge.graph.custom for " .. name)
		print("DEBUG: target = " .. tostring(target))
		print("DEBUG: command = " .. tostring(cargo))
		print("DEBUG: workdir = " .. tostring(forge.project.root))
		print("DEBUG: outputs[1] = " .. tostring(output_path))

		-- Register in the build graph as a custom component
		forge.graph.custom {
			name = name,
			target = target,
			command = cargo,
			args = cargo_args,
			srcs = srcs,
			outputs = { output_path },
			deps = forge_deps,
			env = env,
			workdir = forge.project.root,
		}
	end
end

-- library is similar for now
M.library = M.binary

return M
