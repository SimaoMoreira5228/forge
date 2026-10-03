# bench/

One tiny C program (library + binary + test, identical sources for every
system), described for five build systems side by side, plus a script that
times them all the same way.

## Layout

```
bench/
  measure.sh   # cold / warm / touch / edit / test timing for every system
  app/
    lib/math.c, lib/math.h, src/main.c, tests/math_test.c   # shared sources
    FORGE_ROOT, FORGE.toml   # forge (system gcc, no downloads)
    CMakeLists.txt           # cmake (Ninja generator)
    meson.build              # meson (ninja backend)
    BUILD.bazel, MODULE.bazel  # bazel (needs rules_cc from the network once)
    BUCK, .buckconfig, .buckroot, toolchains/BUCK  # buck2 (bundled prelude)
```

Each tool only reads its own files; output directories differ per system
(`forge-out/`, `build/`, `builddir/`, `buck-out/`, `bazel-*`), so they do
not disturb each other. The app has its own `FORGE_ROOT`, which also keeps
it out of the forge self-host workspace: discovery stops at the first
`FORGE.toml` going down, and the repo root already has one.

## Running

```sh
bench/measure.sh
```

Tool selection: `$FORGE_BIN` (default `target/release/forge` — build it
with `cargo build --release -p forge`), `$BUCK2_BIN`, `$BAZEL_BIN`, else
`PATH`. Missing tools print a skip line and never fail the script.
`CC` defaults to `/usr/bin/gcc`; `CCACHE_DISABLE=1` is forced.

Recorded numbers live in `local/PERFORMANCE.md` (git-ignored scratch),
not here: this folder holds the runnable benchmark, not its results.
