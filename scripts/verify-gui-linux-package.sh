#!/usr/bin/env bash

set -euo pipefail

if (( $# != 2 )); then
  echo "Usage: $0 <version> <artifact-directory>" >&2
  exit 2
fi

version="$1"
artifact_dir="$2"
artifact_root="myr-gui-${version}-linux-x86_64"
archive="${artifact_dir}/${artifact_root}.tar.gz"
appimage="${artifact_dir}/${artifact_root}.AppImage"

if [[ ! -s "${archive}" ]]; then
  echo "Missing GUI archive: ${archive}" >&2
  exit 1
fi

if [[ ! -x "${appimage}" ]]; then
  echo "Missing executable AppImage: ${appimage}" >&2
  exit 1
fi

actual_entries="$(tar -tzf "${archive}" | LC_ALL=C sort)"
expected_entries="$(printf '%s\n' \
  "${artifact_root}/" \
  "${artifact_root}/LICENSE" \
  "${artifact_root}/README.md" \
  "${artifact_root}/myr-gui" \
  | LC_ALL=C sort)"

if [[ "${actual_entries}" != "${expected_entries}" ]]; then
  echo "Unexpected files in ${archive}:" >&2
  printf '%s\n' "${actual_entries}" >&2
  exit 1
fi

APPIMAGE_EXTRACT_AND_RUN=1 "${appimage}" --appimage-version

printf 'Verified %s and %s\n' "${archive}" "${appimage}"
