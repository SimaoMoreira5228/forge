let flags = ["-Wall"];

binary("hello_rhai", #{
    srcs: glob("src/*.c"),
    flags: flags,
});
