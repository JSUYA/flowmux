// SPDX-License-Identifier: GPL-3.0-or-later
//
// Build script for flowmux-terminal.
//
// flowmux's only terminal backend is libghostty-vt, so this always compiles the
// C shim (csrc/ghostty_shim.c) and links it against a static libghostty-vt
// built by scripts/build-ghostty-vt.sh. That means `cargo build`/`cargo check`
// needs Zig 0.15.x on PATH (to build libghostty-vt on the first run) — there is
// no VTE path and no opt-in feature gate any more.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // The libghostty-vt core is always built: compile the C shim and link a
    // static libghostty-vt produced by scripts/build-ghostty-vt.sh (needs Zig).
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // crates/flowmux-terminal -> workspace root.
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("flowmux-terminal lives at <root>/crates/flowmux-terminal")
        .to_path_buf();

    let prefix = resolve_prefix(&workspace_root);
    let include_dir = prefix.join("include");
    let lib_dir = prefix.join("lib");
    let static_lib = lib_dir.join("libghostty-vt.a");
    assert!(
        static_lib.is_file(),
        "expected {} after build-ghostty-vt.sh; got nothing",
        static_lib.display()
    );

    // Compile the stable shim against the pinned libghostty-vt headers.
    cc::Build::new()
        .file(manifest_dir.join("csrc/ghostty_shim.c"))
        .include(manifest_dir.join("csrc"))
        .include(&include_dir)
        .warnings(true)
        .compile("flowmux_ghostty_shim");

    // Link the static libghostty-vt. It is self-contained (the static .pc lists
    // no private deps); pthread/m cover the libc bits the Zig archive expects.
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-lib=dylib=m");
    // forkpty(3) lives in libutil on glibc/BSD; on macOS it is part of
    // libSystem (there is no standalone libutil), so only link it off-macOS.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        println!("cargo:rustc-link-lib=dylib=util");
    }

    // Rebuild triggers.
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("csrc/ghostty_shim.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("csrc/ghostty_shim.h").display()
    );
    println!("cargo:rerun-if-changed={}", static_lib.display());
    println!("cargo:rerun-if-env-changed=FLOWMUX_GHOSTTY_VT_PREFIX");
}

/// Locate (or build) the libghostty-vt install prefix.
///
/// Priority:
/// 1. `FLOWMUX_GHOSTTY_VT_PREFIX` — a pre-built prefix (CI / packaging).
/// 2. `<workspace>/target/ghostty-vt/prefix` — built on demand by
///    scripts/build-ghostty-vt.sh (idempotent; needs Zig + network on the
///    first run only).
fn resolve_prefix(workspace_root: &std::path::Path) -> PathBuf {
    if let Some(p) = env::var_os("FLOWMUX_GHOSTTY_VT_PREFIX") {
        return PathBuf::from(p);
    }

    let default_prefix = workspace_root.join("target/ghostty-vt/prefix");
    if default_prefix.join("lib/libghostty-vt.a").is_file() {
        return default_prefix;
    }

    // Build it. The script is idempotent and prints the prefix on its last line.
    let script = workspace_root.join("scripts/build-ghostty-vt.sh");
    let status = Command::new("bash")
        .arg(&script)
        .arg(&default_prefix)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", script.display()));
    assert!(
        status.success(),
        "{} failed; build libghostty-vt manually or set FLOWMUX_GHOSTTY_VT_PREFIX",
        script.display()
    );
    default_prefix
}
