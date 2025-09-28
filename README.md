# Forge

A Lua-powered build system experiment - because apparently I thought "you know what the world needs? Another build system!"

## What is this thing?

This started as me being way too curious about how build systems actually work. I kept wondering about content-addressed caching, parallel execution, and how tools like Bazel manage to not completely fall apart when dealing with massive codebases. So naturally, I decided to build my own version because that's definitely the most reasonable response to curiosity, right?

**This is absolutely not production-ready software.** It's a learning experiment that happens to work for some basic use cases and will probably break in creative ways you haven't thought of yet.

## Current Status

What actually works right now (surprisingly more than I expected):

- C/C++ Support: Can build C and C++ projects with GCC, Clang, or Zig
- Zig Support: Native Zig libraries, executables, and build.zig wrapper
- Bash/Shell Support: Run scripts, commands, or inline shell code
- Make/CMake Wrappers: Integrate existing Make and CMake projects
- Content-Addressed Caching: Files are cached by hash to avoid rebuilds
- Parallel Execution: Multiple compilation jobs run concurrently
- Lua Configuration: Build scripts are written in Lua (because JSON is boring)
- Multiple Targets: Cross-compilation support
- HTTP Client: Download files from the internet
- JSON/TOML Parsing: Because data formats are everywhere
- Semver Support: Version comparisons that actually work
- Time API: For when you need to know what time it is in your build script
- Hashing Utils: Blake3 and friends for all your hashing needs

What doesn't work yet (the fun stuff):

- Rust Support: The rust prelude is literally just a `.gitkeep` file. Maybe someday!
- Dependency Management: No package resolution (i did give it a shot, but it didn't go well)
- Proper Documentation: You're looking at it, hope you like reading code
- Windows Support: Probably broken, definitely untested
- Error Messages: Often cryptic, occasionally helpful, always an adventure

## Trying it Out

```bash
git clone https://github.com/SimaoMoreira5228/forge.git
cd forge
cargo build --release
```

Check out the examples to see what actually works:

- `examples/c_example/` - Basic C project that actually compiles
- `examples/cpp_example/` - C++ with different standards (fancy!)
- `examples/zig_example/` - Native Zig compilation
- `examples/zig_build_example/` - Wrapping build.zig projects
- `examples/bash_example/` - Shell scripts and command execution
- `examples/zig_raylib_example/` - Zig game with raylib (download, build, link!)
- `examples/make_example/` - Wrapping Makefile-based projects
- `examples/cmake_example/` - Wrapping CMake-based projects
- `examples/minimal_rust_example/` - Rust placeholder (doesn't work, see above)
- `examples/complex_rust_example/` - Another Rust example (also doesn't work, but with more files!)

Build an example:

```bash
cd examples/c_example
../../target/release/forge build
```

If it works, congratulations! If it doesn't, well... that's part of the learning experience! 🎉

### Content-Addressed Storage

Every file gets hashed with Blake3. If the hash exists in `forge-out/cas/`, we skip the work. This means identical files across projects share storage, and changes are detected instantly.

### Lua Build Scripts

Build files are Lua scripts. This gives you real programming constructs (loops, conditions, functions) without needing a complex DSL.

```lua
local c = require("@prelude/c/c.lua")

c.binary({
    name = "my_app",
    srcs = forge.fs.glob("src/**/*.c"),
    compiler = "gcc",
    flags = { "-O2", "-Wall" },
})
```

Plus, you get access to a bunch of built-in APIs for when your build scripts inevitably need to do weird things:

```lua
-- Get stuff from the internet
local content = forge.http.get("https://api.github.com/repos/SimaoMoreira5228/forge")

-- Parse some JSON while you're at it
local data = forge.parse.json(content.body)

-- Check if versions make sense
local compatible = forge.semver.satisfies("1.2.3", "^1.0")

-- Hash everything (we love hashing)
local hash = forge.hash.blake3_file("important_file.txt")
```

### Parallel Everything

File scanning, dependency resolution, and compilation all happen in parallel where possible. Uses Rayon for work-stealing across CPU cores.

## C/C++ Examples

This is what actually works:

```lua
-- C project with multiple compiler options
local c = require("@prelude/c/c.lua")

c.binary({
    name = "app",
    targets = {
        gcc_debug = { compiler = "gcc" },
        clang_debug = { compiler = "clang" },
        zig_debug = { compiler = "zig" },
    },
    srcs = forge.fs.glob("src/**/*.c"),
})
```

```lua
-- C++ with different standards
local cpp = require("@prelude/cpp/cpp.lua")

cpp.binary({
    name = "modern_app",
    targets = {
        cpp17 = { compiler = "gcc", standard = "c++17" },
        cpp20 = { compiler = "clang", standard = "c++20" },
    },
    srcs = forge.fs.glob("src/**/*.cpp"),
})
```

## Why Build This?

Honestly? I was curious and had free time. I wanted to understand:

- How content-addressed caching works (like in Nix, but simpler)
- What makes build systems fast or slow (spoiler: mostly I/O)
- How to design APIs that don't make you want to throw your laptop out the window
- Whether Rust + Lua is actually a decent combo (verdict: surprisingly yes!)

Plus, existing build tools are either too simple (make) or too complex (Bazel). I wanted something in between that I could actually understand and didn't require a PhD in build system archaeology to use.

## File Structure

```
forge-out/
├── cas/                    # Content-addressed storage
│   └── <hash>/            # Cached build artifacts
├── cache.json             # Build metadata
└── <target>/              # Target-specific outputs
```

## Commands

```bash
# Build commands
forge build --target <target>                        # Build specific target(s)
forge build --target <target1> --target <target2>    # Build multiple targets
forge build --component <component>                  # Build specific component(s)
forge build --component <comp1> --component <comp2>  # Build multiple components
forge build --component <component> --target <target> # Combine component and target filters

# Run commands
forge run                                            # Build and run (if binary)
forge run --target <target>                         # Run specific target
forge run --component <component>                    # Run specific component

# Test commands
forge test --target <target>                        # Run tests for specific target (required)
forge test --target <target> --component <component> # Test specific component

# Project setup commands
forge init                                          # Initialize a new forge project
forge init --name <name>                            # Initialize with custom name
forge init --force                                  # Force overwrite existing FORGE_ROOT
forge migrate                                       # Migrate existing project to FORGE_ROOT format
forge migrate --force                               # Force overwrite during migration

# Development commands
forge types                                         # Generate Lua type definitions (types.lua)
forge types --output <path>                         # Generate types to custom path

# Other commands
forge clean                                          # Delete forge-out/

# Examples:
forge --target linux_x64_debug                      # Build debug target (no subcommand)
forge --target linux_x64_release                    # Build release target (no subcommand)
forge build --component math_utils                  # Build only the math_utils library
forge build --component calc                        # Build calc binary and its dependency
forge build --target linux_x64_debug                # Build all components for linux_x64_debug target
forge build --component calc --target linux_x64     # Build calc component for linux_x64 target only
forge test --target linux_x64_debug                 # Run all tests for debug target
```

## What I Learned

Some interesting discoveries from building this (besides "build systems are harder than they look"):

- Blake3 is stupidly fast - Hashing files is barely a bottleneck, even with huge codebases
- Lua embedding is surprisingly nice - Much easier than I expected with mlua
- Rayon makes parallelism trivial - Work-stealing just works, who knew?
- Content addressing is powerful - But cache invalidation is still hard
- Build systems are mostly file copying - With some shell commands sprinkled in for flavor
- APIs are hard to design - Half of these functions probably shouldn't exist
- Rust dependency resolution is nightmare fuel - And that's why the rust prelude is empty

## Current Issues

Things that definitely need work (aka my TODO list that keeps growing):

- Error messages are terrible (think "something went wrong" level of helpful)
- No proper logging or progress indication (enjoy the silent treatment)
- Cache can grow unbounded (hope you like big directories!)
- Lua API is inconsistent (because consistency is overrated, right?)
- Documentation is this README (and you're already reading the best part)
- The Rust prelude is just a `.gitkeep` file

## Not Planning to Add

Some things I explicitly don't want to implement (because I know my limits):

- Remote caching (too complex and I like my sanity)
- Distributed builds (see above, but with more network headaches)
- Package management (Cargo exists and does this better than I ever could)
- Windows support (I don't have a Windows machine)
- Rust support (the `.gitkeep` file in `prelude/rust/` speaks volumes)

## License

MIT - do whatever you want with this. If it breaks your computer, that's between you and your computer. If it somehow becomes sentient and takes over the world, please don't blame me.

---

_P.S. - If you actually use this for something important, please let me know so I can either be very proud or very worried._
