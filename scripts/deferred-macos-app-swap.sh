#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Replace a staged FlowMux.app only after the running host exits.
# Always exit successfully so launchctl does not restart a failed swap.
set -u

if [ "$#" -ne 4 ]; then
    echo "deferred app swap requires PID, staged, destination, and backup paths" >&2
    exit 0
fi

host_pid="$1"
staged_bundle="$2"
destination_bundle="$3"
backup_bundle="$4"

if [ "$(basename "$staged_bundle")" != ".FlowMux.app.pending" ] || \
    [ "$(basename "$destination_bundle")" != "FlowMux.app" ] || \
    [ "$(basename "$backup_bundle")" != ".FlowMux.app.previous" ]; then
    echo "refusing deferred app swap with unexpected bundle paths" >&2
    exit 0
fi

while kill -0 "$host_pid" 2>/dev/null; do
    sleep 0.2
done

if [ ! -d "$staged_bundle" ]; then
    echo "deferred app swap missing staged bundle: $staged_bundle" >&2
    exit 0
fi

rm -rf "$backup_bundle"
if [ -e "$destination_bundle" ] && ! mv "$destination_bundle" "$backup_bundle"; then
    echo "deferred app swap could not move current bundle: $destination_bundle" >&2
    exit 0
fi

if mv "$staged_bundle" "$destination_bundle"; then
    rm -rf "$backup_bundle"
else
    echo "deferred app swap could not install: $staged_bundle" >&2
    if [ -e "$backup_bundle" ]; then
        mv "$backup_bundle" "$destination_bundle"
    fi
fi
exit 0
