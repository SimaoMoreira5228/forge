local compiler_common = require("@prelude/compiler_common.lua")

local M = {}

local to_absolute_path = compiler_common.to_absolute_path

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

M.script = M.run_script
M.command = M.run_command
M.inline = M.run_inline

return M
