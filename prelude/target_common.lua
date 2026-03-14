local M = {}

local function load_targets()
	local targets_list = forge.target:list()
	local predefined = {}
	for _, t in ipairs(targets_list) do
		predefined[t.name] = t
	end
	return predefined
end

M.canonical_targets = load_targets()

function M.get_target_directory(target_name, variant_name)
	local target_info = M.canonical_targets[target_name]
	if not target_info then
		local resolved = forge.target:resolve(target_name)
		if not resolved then
			error(("Unknown target: %s"):format(target_name))
		end
		if variant_name then
			return variant_name
		else
			return resolved.triple
		end
	end

	if variant_name then
		return variant_name
	else
		return target_info.triple or target_info.canonical_name
	end
end

function M.extract_base_target(variant_name)
	local base = variant_name:match("^([^_]+_[^_]+)")
	if base and M.canonical_targets[base] then
		return base
	end

	if M.canonical_targets[variant_name] then
		return variant_name
	end

	return variant_name
end

function M.get_target_info(target_name)
	return M.canonical_targets[target_name] or forge.target:resolve(target_name)
end

function M.get_canonical_triple(target_name)
	local target_info = M.canonical_targets[target_name]
	if not target_info then
		local resolved = forge.target:resolve(target_name)
		if not resolved then
			error(("Unknown target: %s"):format(target_name))
		end
		return resolved.triple
	end
	return target_info.triple or target_info.canonical_name
end

function M.get_structured_target(target_name)
	local target_info = M.canonical_targets[target_name]
	if not target_info then
		local resolved = forge.target:resolve(target_name)
		if not resolved then
			error(("Unknown target: %s"):format(target_name))
		end
		return resolved
	end
	return {
		arch = target_info.arch,
		os = target_info.os,
		abi = target_info.abi,
		vendor = target_info.vendor,
	}
end

function M.triple_to_target_name(triple)
	for name, info in pairs(M.canonical_targets) do
		local canonical = info.triple or info.canonical_name
		if canonical == triple then
			return name
		end
	end
	return nil
end

function M.get_available_targets()
	local targets = {}
	for name, _ in pairs(M.canonical_targets) do
		table.insert(targets, name)
	end
	table.sort(targets)
	return targets
end

function M.validate_target(target_name)
	if not M.canonical_targets[target_name] then
		local resolved = forge.target:resolve(target_name)
		if not resolved then
			local available = M.get_available_targets()
			error(("Unknown target '%s'. Available targets: %s"):format(target_name, table.concat(available, ", ")))
		end
	end
	return true
end

function M.get_predefined_targets()
	return M.canonical_targets
end

return M
