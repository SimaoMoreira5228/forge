use std::env;
use std::path::PathBuf;

fn main() {
	println!("cargo:rerun-if-changed=lib/math_native/");

	println!("cargo:rustc-link-lib=static=math_native");

	let out_dir = env::var("OUT_DIR").unwrap();
	let profile = env::var("PROFILE").unwrap();
	let target = env::var("TARGET").unwrap();

	let lib_dir = format!("forge-out/{}/{}", target, profile);
	println!("cargo:rustc-link-search=native={}", lib_dir);

	println!("cargo:rustc-link-search=native=forge-out/x86_64-unknown-linux-gnu/debug");
	println!("cargo:rustc-link-search=native=forge-out/x86_64-unknown-linux-gnu/release");

	if cfg!(feature = "native-math") {
		println!("cargo:rustc-cfg=feature=\"native-math\"");
	}

	if cfg!(feature = "crypto") {
		println!("cargo:rustc-cfg=feature=\"crypto\"");
	}

	if cfg!(feature = "benchmark-mode") {
		println!("cargo:rustc-cfg=feature=\"benchmark-mode\"");
		println!("cargo:rustc-env=RUSTFLAGS=-C target-cpu=native");
	}
}
