//! Builds libmgba (from this fork's sources, three directories up) as a static
//! library with the vendored zlib/libpng, then compiles the tiny C shim that
//! the Rust side binds to.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../..").canonicalize().expect("mGBA source root");

    // Only watch the parts of the tree the core is built from (watching all of
    // `src/` would include this crate's own `target/` directory).
    for dir in [
        "CMakeLists.txt",
        "include",
        "src/arm",
        "src/core",
        "src/gb",
        "src/gba",
        "src/sm83",
        "src/util",
        "src/feature",
        "src/platform/posix",
        "src/platform/dioxus/shim",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }

    let dst = cmake::Config::new(&root)
        .define("LIBMGBA_ONLY", "ON")
        .define("LIBMGBA_BUNDLED_PNG", "ON")
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("CMAKE_POLICY_VERSION_MINIMUM", "3.5")
        .profile("Release")
        .build_target("mgba")
        .build();
    let build = dst.join("build");

    // `struct mCore` (and friends) change shape depending on feature macros, and
    // not all of them end up in the generated flags.h. Compile the shim with the
    // exact definitions CMake used for libmgba so both sides agree on layouts.
    let mut shim = cc::Build::new();
    for def in cmake_defines(&build) {
        match def.split_once('=') {
            Some((k, v)) => shim.define(k, Some(v)),
            None => shim.define(&def, None),
        };
    }
    shim.file(manifest.join("shim/mgba_shim.c"))
        .include(root.join("include"))
        .include(root.join("src"))
        .include(build.join("include"))
        .opt_level(3)
        .warnings(false)
        .compile("mgba_shim");

    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=static=mgba");
    link_found(&build, "libpng", "png16");
    link_found(&build, "zlib", "z");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}

/// Read the `-D` definitions CMake used for the `mgba` target (Makefile or Ninja generators).
fn cmake_defines(build: &Path) -> Vec<String> {
    let mut text = std::fs::read_to_string(build.join("CMakeFiles/mgba.dir/flags.make"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("C_DEFINES"))
                .map(str::to_owned)
        });
    if text.is_none() {
        // Ninja: definitions live per-object in build.ninja.
        text = std::fs::read_to_string(build.join("build.ninja")).ok().and_then(|t| {
            t.lines()
                .find(|l| l.trim_start().starts_with("DEFINES") && l.contains("M_CORE_GBA"))
                .map(str::to_owned)
        });
    }
    let text = text.expect("could not find libmgba compile definitions");
    text.split_whitespace()
        .filter_map(|tok| tok.strip_prefix("-D"))
        .map(str::to_owned)
        .collect()
}

/// Link a vendored static lib if CMake produced it, otherwise fall back to the system one.
fn link_found(build: &Path, subdir: &str, name: &str) {
    let dir = build.join(subdir);
    if dir.join(format!("lib{name}.a")).exists() {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static={name}");
    } else {
        println!("cargo:rustc-link-lib={name}");
    }
}
