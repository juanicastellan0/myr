#!/usr/bin/env bash

set -euo pipefail

readonly LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
readonly LINUXDEPLOY_SHA256="421ca71d5c69ea97c6309276232990d43df1dcece0edfaa26bbf926ff96ed12e"

destination="${1:-tools/linuxdeploy-x86_64.AppImage}"
destination_dir="$(dirname -- "${destination}")"
partial="${destination}.part"

mkdir -p -- "${destination_dir}"

cleanup() {
  rm -f -- "${partial}"
}
trap cleanup EXIT

curl --fail --location --retry 3 \
  --output "${partial}" \
  "${LINUXDEPLOY_URL}"

printf '%s  %s\n' "${LINUXDEPLOY_SHA256}" "${partial}" | sha256sum --check --status -
chmod 0755 "${partial}"
mv -- "${partial}" "${destination}"
trap - EXIT

printf 'Downloaded verified linuxdeploy to %s\n' "${destination}"
