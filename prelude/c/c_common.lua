local M = {}

local targets_list = forge.target:list()
local predefined = {}
for _, t in ipairs(targets_list) do
	predefined[t.name] = t
end
M.predefined_targets = predefined

M.compilers = {
	gcc = "gcc",
	clang = "clang",
	zig = "zig cc",
}

function M.get_host_target()
	return forge.target:host()
end

function M.get_target_triple_string(target)
	if type(target) == "table" and target.triple then
		return target.triple
	end
	if type(target) == "string" then
		local resolved = forge.target:resolve(target)
		if resolved and resolved.triple then
			return resolved.triple
		end
	end
	return target.canonical_name or tostring(target)
end

function M.resolve_sources(sources, base_path)
	local result = forge.source.resolve({ patterns = sources })
	return result
end

function M.resolve_includes(includes, base_path)
	local result = forge.source.includes({ dirs = includes })
	return result
end

function M.get_target_directory(target_name, variant_name)
	if variant_name then
		return variant_name
	end
	
	local resolved = forge.target:resolve(target_name)
	if resolved and resolved.triple then
		return resolved.triple
	end
	
	return target_name
end

function M.get_compiler_for_target(compiler, target, compiler_path)
	local args = {}

	if compiler_path then
		return { command = compiler_path, args = {} }
	end

	if compiler == "zig" then
		local target_info = type(target) == "table" and target or forge.target:resolve(target)
		local zig_target = target_info and target_info.triple or "x86_64-linux-gnu"
		return { command = "zig", args = { "cc", "-target", zig_target } }
	elseif compiler == "clang" then
		local clang_target = M.get_target_triple_string(target)
		return { command = "clang", args = { "--target=" .. clang_target } }
	else
		local gcc_cmd = "gcc"
		return { command = gcc_cmd, args = {} }
	end
end

return M
