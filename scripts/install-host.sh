#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Release-build flowmux and install the binaries to the host.
#
# flowmux renders terminals with the libghostty-vt backend, which is linked as
# a static library built from a pinned Ghostty revision by
# scripts/build-ghostty-vt.sh (invoked automatically by flowmux-terminal's
# build.rs on the first build). So the only extra prerequisite beyond the GTK4 /
# libadwaita / WebKitGTK dev packages is **Zig 0.15.x on PATH**. No patched VTE
# is needed any more — this is a plain `cargo build --release`.
#
# Usage: scripts/install-host.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v zig >/dev/null 2>&1; then
    echo "error: zig not found on PATH — flowmux-terminal builds libghostty-vt with Zig 0.15.x" >&2
    echo "       install Zig 0.15.x (https://ziglang.org/download/) and retry" >&2
    exit 1
fi

echo "==> building flowmux (release); libghostty-vt is built/linked by build.rs"
cargo build --release -p flowmux -p flowmux-cli

for dir in "$HOME/.local/bin" "$HOME/.cargo/bin"; do
    if [ -d "$dir" ]; then
        install -m755 target/release/flowmux target/release/flowmuxctl "$dir/"
        echo "==> installed to $dir"
    fi
done

echo "==> done. Fully restart the running flowmux GUI to pick up the new binary."
