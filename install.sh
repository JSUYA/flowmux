#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Release-build flowmux and install the binaries to the host.
#
# flowmux uses the system GTK4/libadwaita/WebKitGTK/VTE libraries. The image
# viewer links the system ThorVG (via the thorvg-sys crate in pkg-config mode),
# so ThorVG must be installed first — see scripts/install-thorvg.sh.
#
# Usage: ./install.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

if ! pkg-config --exists thorvg-1 && ! pkg-config --exists thorvg; then
    echo "error: system ThorVG not found (pkg-config: thorvg-1)." >&2
    echo "       The image viewer links ThorVG. Install it first with:" >&2
    echo "         scripts/install-thorvg.sh" >&2
    echo "       (or set PKG_CONFIG_PATH if ThorVG is in a custom prefix)." >&2
    exit 1
fi

echo "==> building flowmux (release)"
cargo build --release -p flowmux -p flowmux-cli -p flowmux-md-viewer

for dir in "$HOME/.local/bin" "$HOME/.cargo/bin"; do
    if [ -d "$dir" ]; then
        install -m755 \
            target/release/flowmux \
            target/release/flowmuxctl \
            target/release/flowmux-md-viewer \
            "$dir/"
        echo "==> installed to $dir"
    fi
done

echo "==> done. Fully restart the running flowmux GUI to pick up the new binary."
