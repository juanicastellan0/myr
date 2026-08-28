# Release Process

This project publishes release binaries from Git tags via `.github/workflows/release.yml`.

## Preconditions

- `main` is green in CI.
- Combined line coverage is at least `85%`, while `myr-application` remains at least `90%`.
- The AppImage smoke flow passes on Ubuntu 22.04 and 24.04 under X11 and Wayland.
- `[workspace.package].version` in `Cargo.toml` is the version you intend to release.
- `Formula/myr.rb` and `bucket/myr.json` are synced to the release version/revision.
- You have push permission for tags on the repository.

## Create a Release

1. Verify local quality gates:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

2. Sync Homebrew/Scoop install channels to the current commit:

```bash
version="$(awk '
  /^\[workspace\.package\]$/ { in_section=1; next }
  /^\[/ { in_section=0 }
  in_section && $1=="version" {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' Cargo.toml)"
scripts/update-install-channels.sh "${version}" "$(git rev-parse HEAD)"
git add Formula/myr.rb bucket/myr.json
```

3. Create and push an annotated tag that matches the workspace version (the first GUI alpha is shown):

```bash
git tag -a v0.2.0-alpha.1 -m "v0.2.0-alpha.1"
git push origin v0.2.0-alpha.1
```

## What the Workflow Does

- Validates tag format: `v<semver>`.
- Validates tag/version match against `[workspace.package].version`.
- Supports rerunning an existing tag through `workflow_dispatch` and checks out that exact tag.
- Builds `myr-app` in release mode with `--locked`.
- Builds `myr-gui` on Ubuntu 22.04 x86_64 and packages it with `linuxdeploy`.
- Produces archives for:
  - `linux-x86_64`
  - `linux-aarch64`
  - `macos-x86_64`
  - `macos-aarch64`
  - `windows-x86_64`
- Packages each archive with:
  - `myr-app` / `myr-app.exe` (Windows)
  - `README.md`
  - `LICENSE`
- Publishes a GitHub Release with generated notes and:
  - `*.tar.gz` and `*.zip` artifacts
  - `myr-gui-<version>-linux-x86_64.AppImage`
  - `myr-gui-<version>-linux-x86_64.tar.gz`
  - `SHA256SUMS.txt`

The GUI alpha does not add Windows, macOS, or ARM64 GUI artifacts. Existing TUI/CLI artifacts remain cross-platform.

## Install Channels

- Homebrew tap formula is stored at `Formula/myr.rb`.
- Scoop bucket manifest is stored at `bucket/myr.json`.
- Both files are generated/updated via `scripts/update-install-channels.sh`.

## If a Release Fails

- Open the failed run in GitHub Actions and fix the reported issue.
- If the tag is wrong, delete and recreate it:

```bash
git tag -d v0.2.0-alpha.1
git push origin :refs/tags/v0.2.0-alpha.1
```
