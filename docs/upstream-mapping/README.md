# Upstream feature mapping

A living matrix of cmux features (drawn from cmux's public README and
docs at cmux.com/docs) and their flowmux status. Each row links to a
per-feature document with the behavioral spec we reimplement against —
written from public documentation, not from upstream source.

`status` values:

- ✅ — implemented
- 🟡 — in progress
- ⏳ — planned, design-only
- ⛔ — won't port (macOS-specific / out of scope)

| cmux feature                      | flowmux status | Linux replacement                         | Spec                                      |
|-----------------------------------|--------------|-------------------------------------------|-------------------------------------------|
| Domain model (workspace/surface/pane) | 🟡        | same model, recursive split tree          | [domain-model.md](domain-model.md)        |
| `cmux.json` custom commands       | 🟡           | identical schema                          | [config.md](config.md)                    |
| Ghostty config compatibility      | 🟡           | read `~/.config/ghostty/config`           | [config.md](config.md)                    |
| Notification rings (pane border)  | ⏳           | gtk CSS class on pane frame               | [notifications.md](notifications.md)      |
| OSC 9 / 99 / 777 detection        | 🟡           | parser in `flowmux-notify::osc`             | [notifications.md](notifications.md)      |
| Desktop notifications             | 🟡           | `org.freedesktop.Notifications` via zbus  | [notifications.md](notifications.md)      |
| Notification panel (sidebar)      | ⏳           | adw.NavigationSplitView right pane         | [notifications.md](notifications.md)      |
| `flowmux notify` CLI                | 🟡           | clap binary → IPC                         | [ipc.md](ipc.md)                          |
| Vertical tabs / sidebar           | ⏳           | adw.NavigationView + custom row widget    | [ui.md](ui.md)                            |
| Horizontal/vertical splits        | ⏳           | gtk.Paned recursive                       | [terminal.md](terminal.md)                |
| Terminal rendering                | ⏳           | vte4 (default), libghostty (planned)      | [terminal.md](terminal.md)                |
| In-app browser                    | ⏳           | WebKitGTK 6.0 in a surface                | [browser.md](browser.md)                  |
| Browser scriptable API (a11y/refs)| ⏳           | reimplement on WebKitGTK a11y bus         | [browser.md](browser.md)                  |
| SSH workspaces                    | ⏳           | `russh` async client, port-forward localhost | [ssh.md](ssh.md)                          |
| Drag-image-to-remote (scp upload) | ⏳           | gtk drop-target → russh-sftp              | [ssh.md](ssh.md)                          |
| Claude Code Teams launcher        | ⏳           | spawn helper as horizontal split          | [agents.md](agents.md)                    |
| Browser cookie/session import     | ⏳           | freedesktop secrets + sqlite readers      | [browser.md](browser.md)                  |
| Linked PR / branch in sidebar     | ⏳           | gh CLI subprocess + libgit2-rs            | [git.md](git.md)                          |
| Listening-ports detection         | ⏳           | `/proc/net/tcp` walker scoped to PID tree | [git.md](git.md)                          |
| Sparkle auto-update               | ⛔           | use distro packages (.deb / Flatpak)      | —                                         |
| macOS Keychain                    | ⛔           | libsecret / Secret Service                | [browser.md](browser.md)                  |
| Native macOS app bundle           | ⛔           | `.desktop` + AppImage / Flatpak / .deb    | [packaging.md](packaging.md)              |

## Triage inbox

`scripts/sync-upstream.sh` writes diff reports to
[`_inbox/`](_inbox/). Drain the inbox by:

1. Reading the latest report.
2. Updating the relevant feature doc (or creating a new one).
3. Either implementing the change or scheduling it.
4. Bumping `.upstream-cmux/PINNED` via `scripts/sync-upstream.sh --bump`.

## Writing a feature doc

Each feature doc has the same shape:

```markdown
# <feature name>

## What cmux does (from public docs)
- bullet 1 …

## What flowmux does
- bullet 1 …

## Crates touched
- `flowmux-…`

## Open questions / risks
- …
```

Keep it short. The point is to make our re-implementation auditable
against cmux's public behavior, not to duplicate upstream documentation.
