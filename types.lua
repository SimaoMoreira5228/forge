-- Generated Lua type definitions for Forge APIs
-- This file provides type hints for Lua language servers

---@class Fs
---@field new fun(): any
--- Read file contents as string (path must be absolute)
---@field read fun(path: string): any
--- Write string content to file (path must be absolute)
---@field write fun(path: string, content: string): any
--- Create directory and all parent directories (path must be absolute)
---@field mkdir fun(path: string): any
--- Find files matching glob pattern (pattern must be absolute)
---@field glob fun(pattern: string): any
--- Check if file or directory exists (path must be absolute)
---@field exists fun(path: string): any
--- Get modification time as Unix timestamp (path must be absolute)
---@field mtime fun(path: string): any
--- Copy file from source to destination (both paths must be absolute)
---@field copy fun(src: string, dest: string): any
--- Move/rename file from source to destination (both paths must be absolute)
---@field move_file fun(src: string, dest: string): any
--- Remove file or empty directory (path must be absolute)
---@field remove fun(path: string): any
--- Remove directory and all its contents (path must be absolute)
---@field remove_dir fun(path: string): any
--- Check if path is a file (path must be absolute)
---@field is_file fun(path: string): any
--- Check if path is a directory (path must be absolute)
---@field is_dir fun(path: string): any
--- Walk directory tree (path must be absolute)
---@field walk fun(path: string, options: any?): any
--- Get system temporary directory
---@field temp_dir fun(): any
--- Create temporary file with optional prefix
---@field temp_file fun(prefix: string?): any
--- Extract archive to destination (both paths must be absolute)
---@field extract fun(options: any): any

---@type Fs

---@class Http
---@field new fun(): any
--- Perform HTTP GET request
---@field get fun(request: any): any
--- Perform HTTP POST request
---@field post fun(request: any): any
--- Download and cache a file
---@field download fun(request: any): any

---@type Http

---@class Parse
---@field new fun(): any
--- Parse JSON string
---@field json fun(json_str: string): any
--- Parse TOML string
---@field toml fun(toml_str: string): any

---@type Parse

---@class Exec
---@field new fun(): any
--- Execute command with optional arguments (simple version)
---@field exec fun(command: string, args: string[]?): any
--- Execute command with full configuration table
---@field run fun(options: any): any

---@type Exec

---@class Semver
---@field new fun(): any
--- Parse a version string and return a version table
---@field parse_version fun(version_str: string): any
--- Check if a version satisfies a requirement
---@field satisfies fun(version_str: string, req_str: string): any
--- Compare two versions (-1, 0, 1)
---@field compare fun(version1_str: string, version2_str: string): any
--- Find the highest version in a list that satisfies a requirement
---@field find_best_match fun(versions: string[], req_str: string): any

---@type Semver

---@class Platform
---@field new fun(): any
--- Get operating system
---@field os fun(): any
--- Get architecture
---@field arch fun(): any
--- Check if running on Windows
---@field is_windows fun(): any
--- Check if running on macOS
---@field is_macos fun(): any
--- Check if running on Linux
---@field is_linux fun(): any
--- Get path separator
---@field path_separator fun(): any
--- Get executable extension
---@field exe_extension fun(): any
--- Get current working directory
---@field cwd fun(): any

---@type Platform

---@class Path
---@field new fun(): any
--- Join path components
---@field join fun(components: string[]): any
--- Get directory name (parent directory)
---@field dirname fun(path: string): any
--- Get base name (file name with extension)
---@field basename fun(path: string): any
--- Get file extension
---@field extension fun(path: string): any
--- Get file stem (name without extension)
---@field stem fun(path: string): any
--- Check if path is absolute
---@field is_absolute fun(path: string): any
--- Check if path is relative
---@field is_relative fun(path: string): any
--- Canonicalize path (resolve to absolute path)
---@field canonicalize fun(path: string): any
--- Get absolute path without resolving symlinks
---@field absolute fun(path: string): any
--- Normalize path (remove . and .. components)
---@field normalize fun(path: string): any
--- Get home directory
---@field home fun(): any

---@type Path

---@class String
---@field new fun(): any
--- Split string by delimiter
---@field split fun(input: string, delimiter: string): any
--- Join strings with delimiter
---@field join fun(parts: string[], delimiter: string): any
--- Trim whitespace
---@field trim fun(input: string): any
--- Trim whitespace from start
---@field trim_start fun(input: string): any
--- Trim whitespace from end
---@field trim_end fun(input: string): any
--- Check if string starts with prefix
---@field starts_with fun(input: string, prefix: string): any
--- Check if string ends with suffix
---@field ends_with fun(input: string, suffix: string): any
--- Replace all occurrences of a substring
---@field replace fun(input: string, from: string, to: string): any
--- Convert to lowercase
---@field to_lower fun(input: string): any
--- Convert to uppercase
---@field to_upper fun(input: string): any
--- Check if string contains substring
---@field contains fun(input: string, needle: string): any
--- Shell escape - properly escape arguments for shell commands
---@field escape_shell fun(input: string): any
--- Pad string to specified length on the left
---@field pad_left fun(input: string, length: number, pad_char: string?): any
--- Pad string to specified length on the right
---@field pad_right fun(input: string, length: number, pad_char: string?): any

---@type String

---@class Hash
---@field new fun(): any
--- Hash a file
---@field file fun(path: string): any
--- Hash a string
---@field string fun(content: string): any
--- Hash multiple files into a single hash
---@field files fun(paths: string[]): any
--- Verify file hash
---@field verify fun(path: string, expected_hash: string): any
--- Hash bytes directly
---@field bytes fun(bytes: any[]): any

---@type Hash

---@class Time
---@field new fun(): any
--- Get current Unix timestamp
---@field now fun(): any
--- Get current Unix timestamp with milliseconds
---@field now_millis fun(): any
--- Format timestamp
---@field format fun(timestamp: number, format: string?): any
--- Sleep for specified duration (in seconds)
---@field sleep fun(duration: number): any
--- Start a named timer
---@field start_timer fun(name: string?): any
--- Get elapsed time since timer start
---@field elapsed fun(name: string?): any
--- Calculate duration between two timestamps
---@field since fun(start_time: number?, end_time: number?): any

---@type Time

---@class Log
---@field new fun(): any
--- Info level logging
---@field info fun(message: string): any
--- Warning level logging
---@field warn fun(message: string): any
--- Error level logging
---@field error fun(message: string): any
--- Debug level logging
---@field debug fun(message: string): any
--- Trace level logging
---@field trace fun(message: string): any
--- Progress logging
---@field progress fun(current: number, total: number, message: string?): any
--- Print without newline (useful for progress updates)
---@field print fun(message: string): any
--- Print with newline
---@field println fun(message: string): any

---@type Log

---@class Table
---@field new fun(): any
--- Get the length of a table (counts all key-value pairs) This works for both array-like and map-like tables
---@field length fun(tbl: any): any
--- Check if a table is empty
---@field is_empty fun(tbl: any): any
--- Get all keys from a table
---@field keys fun(tbl: any): any
--- Get all values from a table
---@field values fun(tbl: any): any
--- Check if a table contains a specific key
---@field contains_key fun(tbl: any, key: any): any
--- Merge two tables (second table overwrites values from first on key conflicts)
---@field merge fun(tbl1: any, tbl2: any): any

---@type Table

---@class Project
---@field new fun(): any
--- Resolve a path relative to the project root
---@field resolve fun(path: string, project_root: string): any

---@type Project

---@class Forge
---@field config table Configuration table
---@field fs Fs File system operations (all paths must be absolute)
---@field http Http HTTP operations
---@field parse Parse Parsing operations
---@field exec Exec Command execution operations
---@field semver Semver Semantic versioning operations
---@field platform Platform Platform detection operations
---@field path Path Path manipulation operations
---@field string String String manipulation operations
---@field hash Hash Hashing operations
---@field time Time Time operations
---@field log Log Logging operations
---@field table Table Table operations
---@field project Project Project context and utilities
---@field rule fun(rule: table): nil Add a build rule
---@field sleep fun(seconds: number): nil Sleep for specified seconds

---@class Project
---@field root string Absolute path to project root
---@field resolve fun(path: string): string Convert relative path to absolute (relative to project root)

---@type Forge
forge = nil
