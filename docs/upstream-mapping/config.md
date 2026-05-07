# Configuration

## What cmux does (from public docs)

- `cmux.json` at the project root defines per-project custom commands
  surfaced in the command palette (cmux.com/docs/custom-commands).
- Reads `~/.config/ghostty/config` for fonts/themes/colors so cmux
  matches the user's existing Ghostty look.
- Per-app data (recent workspaces, etc.) lives under
  `~/Library/Application Support/cmux`.

## What flowmux does

- `flowmux-config::cmux_json::CmuxJson` parses the same `cmux.json`.
  Unknown keys are accepted (no `deny_unknown_fields`) so files
  written for newer cmux versions still load.
- `flowmux-config::ghostty::parse` reads `~/.config/ghostty/config`
  read-only. We extract the keys flowmux can act on (font, theme,
  background, foreground, cursor); the rest is preserved in `extras`
  for diagnostics.
- Per-app data follows XDG: `$XDG_CONFIG_HOME/flowmux`,
  `$XDG_DATA_HOME/flowmux`, `$XDG_STATE_HOME/flowmux`.

## Crates touched

- `flowmux-config` — schemas, paths, parsers

## Open questions / risks

- The cmux.json schema may grow non-trivial fields (e.g. argument
  prompts, conditional commands). When that lands we re-derive the
  serde types from the public spec rather than from upstream source.
