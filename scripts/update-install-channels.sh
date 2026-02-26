#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/update-install-channels.sh <version> <revision> [owner/repo]

Examples:
  scripts/update-install-channels.sh 0.1.1 "$(git rev-parse HEAD)"
  scripts/update-install-channels.sh 0.1.1 "$(git rev-parse HEAD)" juanicastellan0/myr
EOF
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

derive_repo_from_origin() {
  local origin
  origin="$(git -C "${repo_root}" config --get remote.origin.url || true)"
  if [[ -z "${origin}" ]]; then
    return 1
  fi

  origin="${origin%.git}"
  origin="${origin#git@github.com:}"
  origin="${origin#https://github.com/}"
  origin="${origin#http://github.com/}"

  if [[ "${origin}" != */* ]]; then
    return 1
  fi

  printf '%s\n' "${origin}"
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 2 || $# -gt 3 ]]; then
  usage
  exit 1
fi

version="$1"
revision="$2"
repo="${3:-}"

if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Version must be semver-like (for example: 0.1.1)." >&2
  exit 1
fi

if [[ ! "${revision}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "Revision must be a 40-character git SHA." >&2
  exit 1
fi

if [[ -z "${repo}" ]]; then
  if ! repo="$(derive_repo_from_origin)"; then
    echo "Could not derive owner/repo from remote.origin.url. Pass it explicitly." >&2
    exit 1
  fi
fi

if [[ ! "${repo}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "Repository must be in owner/repo format." >&2
  exit 1
fi

repo_url="https://github.com/${repo}"

mkdir -p "${repo_root}/Formula" "${repo_root}/bucket"

cat > "${repo_root}/Formula/myr.rb" <<EOF
class Myr < Formula
  desc "Terminal-first MySQL/MariaDB schema and data explorer"
  homepage "${repo_url}"
  url "${repo_url}.git",
      tag: "v${version}",
      revision: "${revision}"
  license "MIT"
  head "${repo_url}.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", *std_cargo_args(path: "app")
  end

  test do
    assert_match "Usage:", shell_output("#{bin}/myr-app --help")
  end
end
EOF

cat > "${repo_root}/bucket/myr.json" <<EOF
{
  "version": "${version}",
  "description": "Terminal-first MySQL/MariaDB schema and data explorer",
  "homepage": "${repo_url}",
  "license": "MIT",
  "depends": "rustup",
  "url": "${repo_url}/archive/refs/tags/v${version}.zip",
  "hash": "skip",
  "pre_install": [
    "\$sourceRoot = Get-ChildItem -Path \$dir -Directory | Select-Object -First 1",
    "if (-not \$sourceRoot) { throw 'Could not locate extracted source directory.' }",
    "Push-Location \$sourceRoot.FullName",
    "\$env:CARGO_TARGET_DIR = \\"\$dir\\\\target\\"",
    "cargo install --locked --path app --root \\"\$dir\\"",
    "Pop-Location"
  ],
  "bin": "bin\\\\myr-app.exe",
  "checkver": {
    "github": "${repo_url}"
  },
  "autoupdate": {
    "url": "${repo_url}/archive/refs/tags/v\$version.zip",
    "hash": "skip"
  },
  "notes": [
    "Installs from source and compiles locally.",
    "First install can take a few minutes while Cargo builds dependencies."
  ]
}
EOF

echo "Updated Formula/myr.rb and bucket/myr.json for v${version} (${revision})."
