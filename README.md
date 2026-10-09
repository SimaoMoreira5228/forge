# Forge

Forge is a local build system written in Rust. You describe targets in TOML or Rhai, and Forge turns them into a dependency graph, runs the build actions, and caches their results by content. The bundled cells handle C, C++, and Rust; you can write your own cells for other tools.

I started this to understand build systems. It has grown into something I might actually use, so correctness and useful error messages matter more now. It's still experimental. Expect rough edges and breaking changes while the configuration and APIs settle.

## Try it

The CI builds use Rust 1.98.0. You'll need a Rust toolchain and the usual native build tools for your platform to compile Forge.

```sh
git clone https://github.com/SimaoMoreira5228/forge.git
cd forge
cargo build --release -p forge
```

The binary is at `target/release/forge` (`forge.exe` on Windows). To install it on your PATH instead:

```sh
cargo install --path crates/forge --locked
```

Start with the Cargo-mode Rust example. It uses a pinned Rust toolchain, which Forge downloads into its shared store:

```sh
cd examples/rust-cargo
forge toolchains sync rust
forge build
forge test
```

If you haven't installed the binary, use `../../target/release/forge` in those commands. Run Forge from the example directory so it picks up that example's `FORGE_ROOT`.

## Build files

`FORGE_ROOT` configures the workspace: discovery, toolchains, cells, and build profiles. `FORGE.toml` declares targets in a package. For example, a small C program can use this workspace configuration:

```toml
# FORGE_ROOT
[project]
name = "hello"

[discovery]
include = ["."]

[toolchains.gcc]
from = "path"
path = "/usr"
```

And this target:

```toml
# FORGE.toml
[binary.hello]
srcs = ["src/main.c"]
compiler = "gcc"
```

That toolchain path assumes a Linux machine with GCC installed under `/usr`. You can select a catalog version instead, as the examples do for Rust and other tools. `forge toolchains list` shows the workspace selections; `forge toolchains sync` downloads the configured toolchains.

Use `FORGE.rhai` when a build needs loops, conditions, or custom actions. The language rules themselves live in Rhai cells too. The engine handles the graph, execution, and storage, while the cells decide how to invoke compilers and linkers.

## Commands you'll use

```sh
forge build                          # build the workspace
forge build //:hello                 # build one target and its dependencies
forge build //:hello --profile release
forge run hello                      # build the workspace and run this binary
forge test                           # run tests, or reuse cached verdicts
forge test //:math_test
forge watch --test                   # rebuild and test when files change
forge explain //:hello               # inspect the target's actions and cache keys
forge query 'deps(//:hello)'          # query the dependency graph
forge coverage //:math_test          # collect coverage for the selected graph
forge compile-commands               # write compile_commands.json
forge confine                        # inspect this machine's confinement backend
forge clean                          # remove workspace build outputs
```

Run `forge --help` or `forge <command> --help` for the rest. The CLI also has dependency locking and syncing, build-file formatting, graph export, cache statistics, and build-proof verification and replay.

## Examples

| Directory | What it shows |
| --- | --- |
| [hello](examples/hello) | A C library, executable, and test with TOML targets. Uses a host GCC installation and a pinned Mold linker. |
| [hello-rhai](examples/hello-rhai) | C targets declared in Rhai. |
| [rust-cargo](examples/rust-cargo) | Rust support using a Cargo manifest. |
| [rust-native](examples/rust-native) | Rust targets declared directly in Forge, with registry dependencies. |
| [slint-gui](examples/slint-gui) | A C++ Slint app using either a shared runtime or a runtime built from source and embedded in the binary. |

The Slint source build currently targets Linux and needs extra development packages. Its [README](examples/slint-gui/README.md) covers setup. Embedding Slint removes the separate Slint shared library; the example still links system libraries dynamically. Fully static linking also needs static versions of every dependency.

## Caching and isolation

Forge hashes action inputs and configured toolchains, then stores successful outputs in a content-addressed cache. Build outputs live under `forge-out/`. Downloaded toolchains and dependency data use a per-user store; set `FORGE_STORE_DIR` if you want to choose its location.

Actions receive independent copies of their declared inputs. Writing to those copies cannot overwrite the original workspace files, and Forge publishes only declared outputs. Filesystem restrictions depend on the operating system and available backend, though. Linux can use Landlock, macOS uses Seatbelt, and the Windows Job Object backend does not restrict filesystem access. Check `forge confine` on the machine you're using. Host toolchains and system libraries also limit how reproducible a build is across machines.

An optional HTTP registry can supply prebuilt action results. Builds still run locally, and you don't need a registry to use Forge. Treat a configured registry as a trusted publisher: checksums verify downloaded bytes, not who produced them.

Benchmarks and their setup live in [bench/README.md](bench/README.md).

## License

[MIT](LICENSE).
