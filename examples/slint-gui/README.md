# slint-gui — Slint counter app built by forge

Adapted from the upstream
[slint-cpp-template](https://github.com/slint-ui/slint/tree/master/api/cpp/template)
(CMake). Same program, two linkage variants:

- `forge build //:gui` — links the prebuilt Slint shared library from the
  `slint` toolchain catalog entry. Fast (seconds), needs
  `libslint_cpp.so` next to the binary (staged automatically with
  rpath `$ORIGIN`).
- `forge build //:gui_static` — builds Slint from vendored
  source and embeds it in the binary. No separate Slint library is needed;
  platform libraries such as the C runtime, graphics and windowing libraries
  remain dynamically linked. The destination needs compatible system libraries.
  Slow the first time (the vendored source build takes minutes and cargo
  fetches its dependencies once), then fully cached.

Setup (both variants need the pinned toolchains):

```sh
forge toolchains sync
```

The static variant additionally needs the source tree (git-ignored):

```sh
./fetch-vendor.sh
```

The source build needs development packages for Fontconfig, XKB, Wayland,
EGL, OpenGL and X11/XCB. It fetches Cargo dependencies during the build and
currently targets Linux. Its cached outputs include the static archive and
generated C++ headers.

The C cell accepts produced archives through `metadata.c.static.<name>.lib`.
`link_flags = ["-static"]` requests fully static linking when the toolchain
and all dependencies provide static libraries; this example does not enable it.

Plain `forge build` builds everything, including the Slint source build —
use a selection for the fast variant. Running either binary needs a
display; there is deliberately no test target.
