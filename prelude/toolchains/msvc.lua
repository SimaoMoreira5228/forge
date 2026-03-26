local common = require("@prelude/toolchains/common.lua")

local M = {}

function M.find_vswhere()
	local host = forge.target:host()
	if host.os ~= "windows" then return nil end

	local program_files = os.getenv("ProgramFiles(x86)") or os.getenv("ProgramFiles") or "C:\\Program Files (x86)"
	local vswhere_path = program_files .. "\\Microsoft Visual Studio\\Installer\\vswhere.exe"
	if forge.fs.exists(vswhere_path) then
		return vswhere_path
	end
	return nil
end

function M.find_windows_sdk()
	local base = "C:\\Program Files (x86)\\Windows Kits\\10"
	if not forge.fs.exists(base) then return nil end

	local include_base = base .. "\\Include"
	if not forge.fs.exists(include_base) then return nil end

	local versions = forge.fs.glob(include_base .. "/*")
	if not versions or #versions == 0 then return nil end
	table.sort(versions)
	local latest = versions[#versions]
	local version_name = forge.path.basename(latest)

	return {
		path = base,
		version = version_name,
		include = {
			latest .. "\\ucrt",
			latest .. "\\um",
			latest .. "\\shared",
			latest .. "\\winrt",
		},
		lib = forge.path.join({ base, "Lib", version_name, "um", "x64" }),
		ucrt_lib = forge.path.join({ base, "Lib", version_name, "ucrt", "x64" }),
	}
end

function M.find_msvc_info()
	local vswhere = M.find_vswhere()
	if not vswhere then return nil end

	local result = forge.exec.run({
		command = vswhere,
		args = {
			"-latest",
			"-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
			"-property", "installationPath"
		}
	})

	if result.exit_code == 0 and result.stdout then
		local install_path = result.stdout:match("^%s*(.-)%s*$")
		if install_path and install_path ~= "" then
			local msvc_base = install_path .. "\\VC\\Tools\\MSVC"
			if forge.fs.exists(msvc_base) then
				local entries = forge.fs.glob(msvc_base .. "/*")
				if entries and #entries > 0 then
					table.sort(entries)
					local latest_path = entries[#entries]
					local bin_dir = forge.path.join({ latest_path, "bin", "Hostx64", "x64" })
					if forge.fs.exists(forge.path.join({ bin_dir, "cl.exe" })) then
						local sdk = M.find_windows_sdk()
						local env = {
							INCLUDE = forge.path.join({ latest_path, "include" }),
							LIB = forge.path.join({ latest_path, "lib", "x64" }),
						}
						if sdk then
							for _, inc in ipairs(sdk.include) do
								env.INCLUDE = env.INCLUDE .. ";" .. inc
							end
							env.LIB = env.LIB .. ";" .. sdk.lib .. ";" .. sdk.ucrt_lib
						end
						return {
							bin_dir = bin_dir,
							env = env,
							msvc_path = latest_path,
							sdk = sdk
						}
					end
				end
			end
		end
	end
	return nil
end

function M.sync(name, options)
	local info = M.find_msvc_info()
	if info then
		return info
	end
	return common.sync(name, options)
end

function M.resolve(name, options)
	return common.resolve(name, options)
end

function M.resolve_compiler(name, options)
	local info = M.sync(name, options)
	if not info then
		return nil
	end
	
	local bin_dir = info.bin_dir
	if bin_dir and not forge.fs.exists(common.path_join({ bin_dir, "cl.exe" })) then
		-- Try deep path
		local deep = common.path_join({ bin_dir, "Hostx64", "x64" })
		if forge.fs.exists(common.path_join({ deep, "cl.exe" })) then
			bin_dir = deep
		end
	end

	if not bin_dir then
		return nil
	end

	local cl = common.path_join({ bin_dir, "cl.exe" })
	if forge.fs.exists(cl) then
		return {
			id = "msvc",
			c = cl,
			cpp = cl,
			bin_dir = bin_dir,
			env = info.env,
		}
	end

	return nil
end

return M
