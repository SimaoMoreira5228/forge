local M = {}

function M.path_join(parts)
	return forge.path.join(parts)
end

function M.ensure_dir(path)
	if not forge.fs.exists(path) then
		forge.fs.mkdir(path)
	end
end

function M.normalize_os(os_name)
	if os_name == "macos" then
		return "darwin"
	end
	return os_name
end

function M.host_target_key()
	local host = forge.target:host()
	local os_name = M.normalize_os(host.os or "linux")
	local arch = host.arch or "x86_64"
	return os_name .. "-" .. arch
end

function M.sanitize(value)
	return tostring(value):gsub("[^%w%._-]", "-")
end

function M.url_extension(url)
	local name = forge.path.basename(url)
	if name:match("%.tar%.xz$") then
		return ".tar.xz"
	end
	if name:match("%.tar%.gz$") then
		return ".tar.gz"
	end
	if name:match("%.tgz$") then
		return ".tgz"
	end
	if name:match("%.zip$") then
		return ".zip"
	end
	if name:match("%.tar$") then
		return ".tar"
	end
	return ""
end

function M.toolchain_root()
	return M.path_join({ forge.project.root, ".forge", "toolchains" })
end

function M.downloads_root()
	return M.path_join({ forge.project.root, ".forge", "downloads" })
end

function M.load_catalog()
	local catalog_path = M.path_join({ forge.project.prelude_root, "toolchains", "catalog.toml" })
	local content = forge.fs.read(catalog_path)
	local decoded = forge.toml.decode(content)
	if not decoded.toolchains then
		error("Invalid catalog: missing [toolchains]")
	end
	return decoded.toolchains
end

function M.find_toolchain_entry(catalog, name)
	if catalog[name] then
		return name, catalog[name]
	end

	for canonical_name, data in pairs(catalog) do
		if data.aliases then
			for _, alias in ipairs(data.aliases) do
				if alias == name then
					return canonical_name, data
				end
			end
		end
	end

	return nil, nil
end

function M.apply_version_alias(entry, version)
	if not version then
		return version
	end

	if entry.version_aliases then
		if entry.version_aliases[version] then
			return entry.version_aliases[version]
		end

		local ok, parsed = pcall(forge.semver.parse_version, version)
		if ok and type(parsed) == "table" and parsed.major ~= nil then
			local major_key = tostring(parsed.major)
			if entry.version_aliases[major_key] then
				return entry.version_aliases[major_key]
			end
		end

		local major = tostring(version):match("^(%d+)")
		if major and entry.version_aliases[major] then
			return entry.version_aliases[major]
		end
	end

	return version
end

function M.resolve_version_url(name, version, target_key)
	local catalog = M.load_catalog()
	local canonical_name, entry = M.find_toolchain_entry(catalog, name)
	if not entry then
		error(("Toolchain '%s' not found in prelude/toolchains/catalog.toml"):format(name))
	end
	if not entry.targets or not entry.targets[target_key] then
		error(("No catalog target mapping for '%s' on '%s'"):format(canonical_name, target_key))
	end

	local template = entry.targets[target_key].url
	if not template then
		error(("Catalog entry '%s' target '%s' has no URL"):format(canonical_name, target_key))
	end

	if not version and entry.default_version then
		version = entry.default_version
	end

	local resolved_version = M.apply_version_alias(entry, version)
	if template:find("{version}", 1, true) and not resolved_version then
		error(("Toolchain '%s' requires a version"):format(canonical_name))
	end

	local url = template
	if resolved_version then
		url = url:gsub("{version}", resolved_version)
	end

	return {
		name = canonical_name,
		version = resolved_version,
		url = url,
		target = target_key,
	}
end

function M.find_binary_path(root, binary_name)
	local direct = M.path_join({ root, binary_name })
	if forge.fs.exists(direct) then
		return direct
	end

	local bin_dir = M.path_join({ root, "bin" })
	local bin_direct = M.path_join({ bin_dir, binary_name })
	if forge.fs.exists(bin_direct) then
		return bin_direct
	end

	local candidates = {}
	for _, entry in ipairs(forge.fs.walk(root, { recursive = true })) do
		if not forge.fs.is_dir(entry) and forge.path.basename(entry) == binary_name then
			table.insert(candidates, entry)
		end
	end
	
	if #candidates > 0 then
		table.sort(candidates, function(a, b)
			return #a < #b
		end)
		return candidates[1]
	end

	return nil
end

function M.find_bin_dir(root)
	local direct = M.path_join({ root, "bin" })
	if forge.fs.is_dir(direct) then
		return direct
	end

	local candidates = {}
	for _, candidate in ipairs(forge.fs.walk(root, { recursive = true })) do
		if forge.fs.is_dir(candidate) and forge.path.basename(candidate) == "bin" then
			table.insert(candidates, candidate)
		end
	end
	
	if #candidates > 0 then
		table.sort(candidates, function(a, b)
			return #a < #b
		end)
		return candidates[1]
	end

	return nil
end

function M.extract_archive(archive_path, dest_dir)
	forge.fs.extract({ archive = archive_path, dest = dest_dir })
end

function M.resolve(name, options)
	options = options or {}
	local target_key = options.target_key or M.host_target_key()

	local from = options.from or "version"
	local version = options.version
	local url = options.url
	local sha256 = options.sha256
	local path = options.path
	local git = options.git

	if from == "version" then
		local catalog_resolved = M.resolve_version_url(name, version, target_key)
		url = catalog_resolved.url
		version = catalog_resolved.version
		name = catalog_resolved.name
	end

	local install_key = M.sanitize(name) .. "-" .. M.sanitize(version or "custom") .. "-" .. M.sanitize(target_key)
	local install_dir = M.path_join({ M.toolchain_root(), install_key })

	return {
		name = name,
		from = from,
		version = version,
		url = url,
		sha256 = sha256,
		path = path,
		git = git,
		target_key = target_key,
		install_dir = install_dir,
	}
end

function M.resolve_from_config(name, options)
	options = options or {}
	local toolchain_cfg = ((forge.config or {}).toolchain or {})[name] or {}

	local merged = {
		from = options.from or toolchain_cfg.from,
		version = options.version or toolchain_cfg.version,
		url = options.url or toolchain_cfg.url,
		sha256 = options.sha256 or toolchain_cfg.sha256,
		path = options.path or toolchain_cfg.path,
		git = options.git or toolchain_cfg.git,
		target_key = options.target_key,
	}

	return M.resolve(name, merged)
end

function M.sync(name, options, hooks)
	local spec = M.resolve_from_config(name, options)
	local hooks_tbl = hooks or {}

	M.ensure_dir(M.path_join({ forge.project.root, ".forge" }))
	M.ensure_dir(M.toolchain_root())
	M.ensure_dir(M.downloads_root())

	if spec.from == "path" then
		if not spec.path then
			error(("Toolchain '%s' from=path requires path"):format(name))
		end
		local info = {
			name = name,
			from = spec.from,
			path = spec.path,
			bin_dir = M.find_bin_dir(spec.path),
			available = forge.fs.exists(spec.path),
		}
		if hooks_tbl.after_sync then
			local extra = hooks_tbl.after_sync(spec, info) or {}
			for k, v in pairs(extra) do
				info[k] = v
			end
		end
		return info
	end

	if spec.from == "auto" then
		return {
			name = name,
			from = spec.from,
			path = nil,
			bin_dir = nil,
			available = false,
		}
	end

	if spec.from == "git" then
		if not spec.git or not spec.git.repo or not spec.git.rev then
			error(("Toolchain '%s' from=git requires git.repo and git.rev"):format(name))
		end
		if forge.fs.exists(spec.install_dir) then
			forge.fs.remove_dir(spec.install_dir)
		end
		M.ensure_dir(spec.install_dir)

		local clone = forge.exec.run({
			command = "git",
			args = { "clone", "--depth", "1", spec.git.repo, spec.install_dir },
		})
		if not clone.success then
			error(("Failed to clone git toolchain: %s"):format(clone.stderr or "unknown error"))
		end

		local checkout = forge.exec.run({
			command = "git",
			args = { "checkout", spec.git.rev },
			working_dir = spec.install_dir,
		})
		if not checkout.success then
			error(("Failed to checkout git revision: %s"):format(checkout.stderr or "unknown error"))
		end
	else
		if not spec.url then
			error(("Toolchain '%s' from=%s requires url"):format(name, tostring(spec.from)))
		end

		local archive_name = M.sanitize(name) .. "-" .. M.sanitize(spec.version or "custom") .. M.url_extension(spec.url)
		local marker = M.path_join({ spec.install_dir, ".forge_extracted" })
		if not forge.fs.exists(marker) then
			local archive_path = forge.http.download({
				url = spec.url,
				cache_key = archive_name,
				sha256 = spec.sha256,
				extract = false,
			})

			if forge.fs.exists(spec.install_dir) then
				-- Ignore errors on remove_dir since read-only files might prevent it
				pcall(forge.fs.remove_dir, spec.install_dir)
			end
			M.ensure_dir(spec.install_dir)
			M.extract_archive(archive_path, spec.install_dir)
			forge.fs.write(marker, "ready")
		end
	end

	local info = {
		name = name,
		from = spec.from,
		version = spec.version,
		url = spec.url,
		path = spec.install_dir,
		bin_dir = M.find_bin_dir(spec.install_dir),
		available = forge.fs.exists(spec.install_dir),
	}

	if hooks_tbl.after_sync then
		local extra = hooks_tbl.after_sync(spec, info) or {}
		for k, v in pairs(extra) do
			info[k] = v
		end
	end

	return info
end

function M.list_configured()
	local configured = (forge.config and forge.config.toolchain) or {}
	local out = {}
	for name, cfg in pairs(configured) do
		out[#out + 1] = {
			name = name,
			from = cfg.from,
			version = cfg.version,
			url = cfg.url,
			path = cfg.path,
		}
	end
	table.sort(out, function(a, b)
		return a.name < b.name
	end)
	return out
end

return M
