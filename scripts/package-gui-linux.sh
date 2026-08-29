#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $0 <version> <gui-binary> <linuxdeploy> [output-directory]" >&2
  exit 2
}

if (( $# < 3 || $# > 4 )); then
  usage
fi

version="$1"
binary="$2"
linuxdeploy="$3"
output_dir="${4:-dist/release}"

if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Version '${version}' must be a semantic version without a leading v." >&2
  exit 1
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
workspace_version="$(awk '
  /^\[workspace\.package\]$/ { in_section=1; next }
  /^\[/ { in_section=0 }
  in_section && $1=="version" {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' "${repo_root}/Cargo.toml")"

if [[ "${workspace_version}" != "${version}" ]]; then
  echo "Version '${version}' does not match workspace version '${workspace_version}'." >&2
  exit 1
fi

if [[ ! -x "${binary}" ]]; then
  echo "GUI binary '${binary}' does not exist or is not executable." >&2
  exit 1
fi

if [[ ! -x "${linuxdeploy}" ]]; then
  echo "linuxdeploy '${linuxdeploy}' does not exist or is not executable." >&2
  exit 1
fi

binary="$(realpath -- "${binary}")"
linuxdeploy="$(realpath -- "${linuxdeploy}")"
mkdir -p -- "${output_dir}"
output_dir="$(realpath -- "${output_dir}")"

readonly platform="linux-x86_64"
readonly artifact_root="myr-gui-${version}-${platform}"
readonly archive_path="${output_dir}/${artifact_root}.tar.gz"
readonly appimage_path="${output_dir}/${artifact_root}.AppImage"
readonly source_date_epoch="${SOURCE_DATE_EPOCH:-0}"

if [[ ! "${source_date_epoch}" =~ ^[0-9]+$ ]]; then
  echo "SOURCE_DATE_EPOCH must be a non-negative integer." >&2
  exit 1
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/myr-gui-package.XXXXXX")"
cleanup() {
  rm -rf -- "${work_dir}"
}
trap cleanup EXIT

archive_stage="${work_dir}/archive"
archive_root="${archive_stage}/${artifact_root}"
install -Dm755 "${binary}" "${archive_root}/myr-gui"
install -Dm644 "${repo_root}/README.md" "${archive_root}/README.md"
install -Dm644 "${repo_root}/LICENSE" "${archive_root}/LICENSE"

tar \
  --sort=name \
  --mtime="@${source_date_epoch}" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -C "${archive_stage}" \
  -cf - \
  "${artifact_root}" \
  | gzip -n > "${archive_path}"

appdir="${work_dir}/AppDir"
install -Dm755 "${binary}" "${appdir}/usr/bin/myr-gui"
install -Dm644 \
  "${repo_root}/packaging/linux/io.github.juanicastellan0.myr.desktop" \
  "${appdir}/usr/share/applications/io.github.juanicastellan0.myr.desktop"
install -Dm644 \
  "${repo_root}/packaging/linux/io.github.juanicastellan0.myr.svg" \
  "${appdir}/usr/share/icons/hicolor/scalable/apps/io.github.juanicastellan0.myr.svg"
install -Dm644 \
  "${repo_root}/packaging/linux/io.github.juanicastellan0.myr.metainfo.xml" \
  "${appdir}/usr/share/metainfo/io.github.juanicastellan0.myr.appdata.xml"
install -Dm644 "${repo_root}/README.md" "${appdir}/usr/share/doc/myr/README.md"
install -Dm644 "${repo_root}/LICENSE" "${appdir}/usr/share/doc/myr/LICENSE"

(
  cd -- "${work_dir}"
  ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "${linuxdeploy}" \
    --appdir "${appdir}" \
    --executable "${appdir}/usr/bin/myr-gui" \
    --desktop-file "${repo_root}/packaging/linux/io.github.juanicastellan0.myr.desktop" \
    --icon-file "${repo_root}/packaging/linux/io.github.juanicastellan0.myr.svg" \
    --output appimage
)

mapfile -d '' generated_appimages < <(
  find "${work_dir}" -maxdepth 1 -type f -name '*.AppImage' -print0
)
if (( ${#generated_appimages[@]} != 1 )); then
  echo "Expected linuxdeploy to produce exactly one AppImage; found ${#generated_appimages[@]}." >&2
  exit 1
fi

install -m755 "${generated_appimages[0]}" "${appimage_path}"

printf 'Created %s\n' "${archive_path}"
printf 'Created %s\n' "${appimage_path}"
