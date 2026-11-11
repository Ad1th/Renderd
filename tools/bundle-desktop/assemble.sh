#!/usr/bin/env bash
# assemble.sh - Builds Renderd.app: the desktop app with the host and viewer inside.
#
#   tools/bundle-desktop/assemble.sh            # release build
#   PROFILE=debug tools/bundle-desktop/assemble.sh
#
# The app looks for renderd-host and renderd-viewer beside its own executable,
# so all three go into Contents/MacOS.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

PROFILE="${PROFILE:-release}"
TARGET="${TARGET:-}"

if [ -n "${TARGET}" ]; then
  BIN_DIR="${WORKSPACE_ROOT}/target/${TARGET}/${PROFILE}"
else
  BIN_DIR="${WORKSPACE_ROOT}/target/${PROFILE}"
fi

BUNDLE_DIR="${WORKSPACE_ROOT}/target/bundle/Renderd.app"
MACOS_DIR="${BUNDLE_DIR}/Contents/MacOS"
RESOURCES_DIR="${BUNDLE_DIR}/Contents/Resources"
CRATE_DIR="${WORKSPACE_ROOT}/crates/renderd-desktop"

echo "==> Assembling Renderd.app"
echo "    Binaries:    ${BIN_DIR}"
echo "    Destination: ${BUNDLE_DIR}"

for binary in renderd renderd-host renderd-viewer; do
  if [ ! -f "${BIN_DIR}/${binary}" ]; then
    echo "Error: ${BIN_DIR}/${binary} not found." >&2
    echo "Build it first: cargo build --${PROFILE} -p renderd-desktop -p renderd-host -p renderd-viewer" >&2
    exit 1
  fi
done

rm -rf "${BUNDLE_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

for binary in renderd renderd-host renderd-viewer; do
  cp "${BIN_DIR}/${binary}" "${MACOS_DIR}/${binary}"
  chmod +x "${MACOS_DIR}/${binary}"
done

cp "${CRATE_DIR}/Info.plist" "${BUNDLE_DIR}/Contents/Info.plist"
cp "${CRATE_DIR}/icons/icon.icns" "${RESOURCES_DIR}/icon.icns"

# An ad-hoc signature is enough to run locally and gives macOS a stable
# identity to attach the Screen Recording permission to. Set SIGN_IDENTITY to
# sign with a real certificate.
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign "${SIGN_IDENTITY}" "${BUNDLE_DIR}"
  echo "==> Signed with identity '${SIGN_IDENTITY}'"
fi

echo "==> Renderd.app assembled at ${BUNDLE_DIR}"
