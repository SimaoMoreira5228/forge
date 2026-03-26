local compiler_common = require("@prelude/compiler_common.lua")

local M = {}

local to_absolute_path = compiler_common.to_absolute_path

local function ensure_non_empty_inputs(inputs)
	if #inputs > 0 then
		return inputs
	end
	return { forge.path.join({ forge.project.root, "FORGE" }) }
end

function M.run_script(config)
	if not config.name then
		error("Script configuration must include a 'name' field")
	end

	if not config.script then
		error(("Script '%s' must specify a script file"):format(config.name))
	end

	if not config.outputs or forge.table.length(config.outputs) == 0 then
		forge.log.warn(("Script '%s' has no outputs specified - will always run"):format(config.name))
	end

	local script_path = to_absolute_path(config.script, config.workdir or forge.project.root)

	if not forge.fs.exists(script_path) then
		error(("Script file not found: %s"):format(script_path))
	end

	local args = {}
	if config.args then
		for _, arg in ipairs(config.args) do
			table.insert(args, arg)
		end
	end

	local inputs = { script_path }
	if config.inputs then
		for _, input in ipairs(config.inputs) do
			local abs_input = to_absolute_path(input, config.workdir or forge.project.root)
			table.insert(inputs, abs_input)
		end
	end
	inputs = ensure_non_empty_inputs(inputs)

	local outputs = {}
	if config.outputs then
		for _, output in ipairs(config.outputs) do
			local abs_output = to_absolute_path(output, config.workdir or forge.project.root)
			table.insert(outputs, abs_output)
		end
	end

	local env = {}
	if config.env then
		for key, value in pairs(config.env) do
			env[key] = value
		end
	end

	forge.log.info(("Defining shell script rule '%s'"):format(config.name))

	forge.rule({
		name = config.name,
		command = script_path,
		args = args,
		env = env,
		inputs = inputs,
		outputs = outputs,
		dependencies = config.dependencies or {},
		workdir = config.workdir or forge.project.root,
	})
end

function M.run_command(config)
	if not config.name then
		error("Command configuration must include a 'name' field")
	end

	if not config.command then
		error(("Command '%s' must specify a command"):format(config.name))
	end

	if not config.outputs or forge.table.length(config.outputs) == 0 then
		forge.log.warn(("Command '%s' has no outputs specified - will always run"):format(config.name))
	end

	local args = {}
	if config.args then
		for _, arg in ipairs(config.args) do
			table.insert(args, arg)
		end
	end

	local inputs = {}
	if config.inputs then
		for _, input in ipairs(config.inputs) do
			local abs_input = to_absolute_path(input, config.workdir or forge.project.root)
			table.insert(inputs, abs_input)
		end
	end
	inputs = ensure_non_empty_inputs(inputs)

	local outputs = {}
	if config.outputs then
		for _, output in ipairs(config.outputs) do
			local abs_output = to_absolute_path(output, config.workdir or forge.project.root)
			table.insert(outputs, abs_output)
		end
	end

	local env = {}
	if config.env then
		for key, value in pairs(config.env) do
			env[key] = value
		end
	end

	forge.log.info(("Defining shell command rule '%s': %s"):format(config.name, config.command))

	forge.rule({
		name = config.name,
		command = config.command,
		args = args,
		env = env,
		inputs = inputs,
		outputs = outputs,
		dependencies = config.dependencies or {},
		workdir = config.workdir or forge.project.root,
	})
end

function M.run_inline(config)
	if not config.name then
		error("Inline configuration must include a 'name' field")
	end

	if not config.code then
		error(("Inline command '%s' must specify code"):format(config.name))
	end

	if not config.outputs or forge.table.length(config.outputs) == 0 then
		forge.log.warn(("Inline command '%s' has no outputs specified - will always run"):format(config.name))
	end

	local args = { "-c", config.code }

	local inputs = {}
	if config.inputs then
		for _, input in ipairs(config.inputs) do
			local abs_input = to_absolute_path(input, config.workdir or forge.project.root)
			table.insert(inputs, abs_input)
		end
	end
	inputs = ensure_non_empty_inputs(inputs)

	local outputs = {}
	if config.outputs then
		for _, output in ipairs(config.outputs) do
			local abs_output = to_absolute_path(output, config.workdir or forge.project.root)
			table.insert(outputs, abs_output)
		end
	end

	local env = {}
	if config.env then
		for key, value in pairs(config.env) do
			env[key] = value
		end
	end

	forge.log.info(("Defining inline shell rule '%s'"):format(config.name))

	forge.rule({
		name = config.name,
		command = "bash",
		args = args,
		env = env,
		inputs = inputs,
		outputs = outputs,
		dependencies = config.dependencies or {},
		workdir = config.workdir or forge.project.root,
	})
end

function M.test(config)
	if not config.name then
		error("Test configuration must include a 'name' field")
	end

	-- bash tests run on the host; ensure at least one target is registered.
	-- If no target exists yet, register the host target automatically.
	if forge.graph.target_count() == 0 then
		local os_name   = forge.platform.os()
		local arch_name = forge.platform.arch()
		-- Construct a basic triple: arch-unknown-os-gnu
		local triple
		if os_name == "windows" then
			triple = arch_name .. "-pc-windows-msvc"
		elseif os_name == "macos" then
			triple = arch_name .. "-apple-darwin"
		else
			triple = arch_name .. "-unknown-linux-gnu"
		end
		forge.graph.target({
			name   = "host",
			triple = triple,
		})
	end

	-- Build a proper command list: { "bash", "-c", <script> }
	-- config.args is expected to be: { "-c", "<script>" } or just { "<script>" }
	local args = config.args or {}
	local cmd_list
	if type(args) == "table" and #args >= 1 and args[1] == "-c" then
		-- Already in { "-c", "..." } form — prepend bash
		cmd_list = { "bash" }
		for _, v in ipairs(args) do
			table.insert(cmd_list, v)
		end
	elseif type(args) == "table" and #args >= 1 then
		-- Treat first element as raw shell code
		cmd_list = { "bash", "-c", args[1] }
	elseif type(args) == "string" then
		cmd_list = { "bash", "-c", args }
	else
		error(("Test '%s' must specify args with a shell command"):format(config.name))
	end

	forge.log.info(("Defining shell test '%s'"):format(config.name))

	forge.graph.test({
		name    = config.name .. "_run",
		command = cmd_list,
		env     = config.env or {},
		deps    = config.dependencies or {},
		timeout = config.timeout or 60,
		size    = config.size or "small",
		tags    = config.tags or {},
	})
end

M.script = M.run_script
M.command = M.run_command
M.inline = M.run_inline

return M
