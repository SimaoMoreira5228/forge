local M = {}

M.canonical_targets = {
	windows_x64 = {
		canonical_name = "x86_64-pc-windows-msvc",
		arch = "x86_64",
		os = "windows",
		abi = "msvc",
		vendor = "pc",
	},
	windows_x64_gnu = {
		canonical_name = "x86_64-pc-windows-gnu",
		arch = "x86_64",
		os = "windows",
		abi = "gnu",
		vendor = "pc",
	},
	linux_x64 = {
		canonical_name = "x86_64-unknown-linux-gnu",
		arch = "x86_64",
		os = "linux",
		abi = "gnu",
		vendor = "unknown",
	},
	linux_x64_musl = {
		canonical_name = "x86_64-unknown-linux-musl",
		arch = "x86_64",
		os = "linux",
		abi = "musl",
		vendor = "unknown",
	},
	linux_aarch64 = {
		canonical_name = "aarch64-unknown-linux-gnu",
		arch = "aarch64",
		os = "linux",
		abi = "gnu",
		vendor = "unknown",
	},
	linux_aarch64_musl = {
		canonical_name = "aarch64-unknown-linux-musl",
		arch = "aarch64",
		os = "linux",
		abi = "musl",
		vendor = "unknown",
	},
	macos_x64 = {
		canonical_name = "x86_64-apple-darwin",
		arch = "x86_64",
		os = "macos",
		abi = "macho",
		vendor = "apple",
	},
	macos_aarch64 = {
		canonical_name = "aarch64-apple-darwin",
		arch = "aarch64",
		os = "macos",
		abi = "macho",
		vendor = "apple",
	},
	freebsd_x64 = {
		canonical_name = "x86_64-unknown-freebsd",
		arch = "x86_64",
		os = "freebsd",
		abi = "elf",
		vendor = "unknown",
	},
	wasm32 = {
		canonical_name = "wasm32-unknown-wasi",
		arch = "wasm32",
		os = "wasi",
		abi = "wasm",
		vendor = "unknown",
	},
}

function M.get_target_directory(target_name, variant_name)
	local target_info = M.canonical_targets[target_name]
	if not target_info then
		error(("Unknown target: %s"):format(target_name))
	end

	if variant_name then
		return variant_name
	else
		return target_info.canonical_name
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
	return M.canonical_targets[target_name]
end

function M.get_canonical_triple(target_name)
	local target_info = M.canonical_targets[target_name]
	if not target_info then
		error(("Unknown target: %s"):format(target_name))
	end
	return target_info.canonical_name
end

function M.get_structured_target(target_name)
	local target_info = M.canonical_targets[target_name]
	if not target_info then
		error(("Unknown target: %s"):format(target_name))
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
		if info.canonical_name == triple then
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
		local available = M.get_available_targets()
		error(("Unknown target '%s'. Available targets: %s"):format(target_name, table.concat(available, ", ")))
	end
	return true
end

function M.get_predefined_targets()
	local targets = {}
	for name, canonical_info in pairs(M.canonical_targets) do
		targets[name] = {
			arch = canonical_info.arch,
			os = canonical_info.os,
			abi = canonical_info.abi,
			vendor = canonical_info.vendor,
			canonical_name = canonical_info.canonical_name,
		}
	end
	return targets
end

return M
