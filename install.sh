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

# ThorVG is an optional runtime dependency: flowmux builds and installs without
# it. If it is missing, everything works except the inline image viewer, which
# shows a "ThorVG is not installed" message until the library is present.
if ! ldconfig -p 2>/dev/null | grep -q 'libthorvg-1\.so' \
    && ! pkg-config --exists thorvg-1 2>/dev/null; then
    echo "note: ThorVG not detected — the image viewer will be disabled until" >&2
    echo "      you install it:" >&2
    echo "        sudo apt install libthorvg-dev   # where packaged (e.g. Debian)" >&2
    echo "        scripts/install-thorvg.sh        # build from source (Ubuntu)" >&2
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
