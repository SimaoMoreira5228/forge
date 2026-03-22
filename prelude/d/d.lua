local common = require("@prelude/d/d_common.lua")
local compiler = require("@prelude/d/d_compiler.lua")

local M = {}

M.predefined_targets = common.predefined_targets
M.compilers = common.compilers

local function should_build_target(target_name)
	if forge.config and forge.config.target_filters and #forge.config.target_filters > 0 then
		for _, filter in ipairs(forge.config.target_filters) do
			if filter == target_name then
				return true
			end
		end
		return false
	end
	return true
end

function M.library(tbl)
	forge.log.info(("Defining D library '%s'"):format(tbl.name))
	for target_name, target_config in pairs(tbl.targets) do
		if should_build_target(target_name) then
			compiler.define_library_rules_for_target(tbl, target_name, target_config)
		end
	end
end

function M.binary(tbl)
	forge.log.info(("Defining D binary '%s'"):format(tbl.name))
	for target_name, target_config in pairs(tbl.targets) do
		if should_build_target(target_name) then
			compiler.define_program_rules_for_target(tbl, target_name, target_config)
		end
	end
end

M.executable = M.binary

return M
