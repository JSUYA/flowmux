---
name: flowmux-release
description: Build, locally test, install, version-bump, and push a flowmux release. Use when the user says "빌드하고 설치해줘", "최신으로 빌드 설치", "재설치 해줘", "0.x.y로 업데이트 해줘", "버전 올려서 푸쉬", "릴리즈 CI 실패 검토", or "build and install flowmux". Orchestrates the recurring flowmux ship ritual; does NOT open a PR.
version: 1.0.0
when_to_use: Inside the flowmux Rust workspace, when shipping a change — rebuild the release binaries, install them to the host, optionally bump the workspace version and push, and watch the release CI. Use for "build + install", "reinstall", "bump version + push", and "release CI is failing".
inputs:
  - name: action
    description: One of "install" (build + test + install to host), "release" (install + version bump + push), or "ci" (inspect a failing release run). Defaults to "install".
  - name: version
    description: New semver for "release" (e.g. 0.3.4). Required only when bumping.
required_tools: [bash, read, edit]
---

# flowmux release / install

## Goal

Ship a flowmux change the way the maintainer repeatedly does it: a clean
release build, **tested locally before any push** (hard rule — never push a
build that did not build + test on this host), installed to the host, and —
when releasing — a single-commit version bump pushed to `origin`
(`JSUYA/flowmux`) with the release CI watched green. Never open a PR (the user
opens PRs).

Base build/test/lint commands live in `CLAUDE.md`; this skill orchestrates the
end-to-end ship sequence around them.

## When to use vs. skip

- **Use** for: "빌드하고 설치해줘", "최신 기준 빌드 설치", "재설치", version bump +
  push, and triaging a red release CI run.
- **Skip** for: day-to-day type-checking (`cargo check`), debugging a single
  crate, or editing browser/agent code — those don't need the full ship
  sequence. Skip the Flatpak step unless targeting Ubuntu 22.04.

## Preconditions

- Run from the flowmux repo root (a Cargo workspace; `crates/` present).
- `zig` (0.15.x) on PATH — `flowmux-terminal` compiles vendored
  `libghostty-vt` with it. GTK4 + libadwaita + WebKitGTK 6.0 dev packages for
  the GUI crate.
- `gh` authenticated for the `ci` action and for watching CI after push.

## Procedure

1. **Release build.** `cargo build --release --workspace`. Expect
   `target/release/flowmux` and `target/release/flowmuxctl`. Fix any build
   error here — do not proceed on a broken build.
2. **Test + lint locally (the push gate).**
   `cargo fmt --all --check` →
   `cargo clippy --workspace --all-targets -- -D warnings` →
   `xvfb-run -a dbus-run-session -- cargo test --workspace --locked`
   (headless crates also pass with plain `cargo test -p flowmux-core` etc.).
   All green is the precondition for any push.
3. **Install to host.** `install -m755 target/release/flowmux target/release/flowmuxctl ~/.local/bin/`
   (the maintainer runs `~/.local/bin/flowmux`). Then `flowmux doctor`; run
   `flowmux fix` if doctor flags hooks/SKILL files/socket/browser dir.
4. **(Ubuntu 22.04 / Flatpak only)** Rebuild the sandboxed app:
   `flatpak-builder --user --install --force-clean build-flatpak packaging/flatpak/com.flowmux.App.yml`
   then `flatpak run com.flowmux.App`. Keep 22.04 compatibility intact — it is
   a standing requirement.
5. **Version bump (action = release).** `grep -rn '^version' Cargo.toml crates/*/Cargo.toml`
   to locate the version source; bump `version` in the root
   `[workspace.package]` (crates inherit via `version.workspace = true`). Bump
   any crate that pins its own version too. Re-run step 1 so the binaries carry
   the new version.
6. **Commit + push (action = release).** One conventional commit. Per global
   rules: default git author, **no** issue/PR links in the message, and **do
   not create a PR**. `git push origin <branch>` (default `main`).
7. **Watch release CI.** `gh run list --limit 5` then `gh run watch <id>`. If
   red, `gh run view <id> --log-failed`, fix locally (back to step 1), and
   push the fix.

## Validation

`~/.local/bin/flowmux --version` prints the new version and `flowmux doctor`
exits 0. If pushed: the `JSUYA/flowmux` release run for the pushed commit is
green.

## Output

Installed `flowmux`/`flowmuxctl` on the host (and/or rebuilt Flatpak); for a
release, a pushed single commit with the bumped version and a green CI run.

## Stop condition

Done when the installed binary reports the expected version, `doctor` is clean,
and (if pushed) CI is green. On a build/test/clippy failure: fix locally and
re-run — never push past the step-2 gate. On red CI: pull the failed log, fix,
re-push.
