#!/usr/bin/env bash
# assemble.sh — Assembles renderd-host.app macOS bundle directory structure.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

PROFILE="${PROFILE:-release}"
TARGET="${TARGET:-}"

if [ -n "${TARGET}" ]; then
  BINARY_PATH="${WORKSPACE_ROOT}/target/${TARGET}/${PROFILE}/renderd-host"
else
  BINARY_PATH="${WORKSPACE_ROOT}/target/${PROFILE}/renderd-host"
fi

# Fallback to debug if release binary does not exist
if [ ! -f "${BINARY_PATH}" ]; then
  if [ -f "${WORKSPACE_ROOT}/target/debug/renderd-host" ]; then
    BINARY_PATH="${WORKSPACE_ROOT}/target/debug/renderd-host"
  fi
fi

BUNDLE_DIR="${WORKSPACE_ROOT}/target/bundle/renderd-host.app"
CONTENTS_DIR="${BUNDLE_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

echo "==> Assembling renderd-host.app bundle"
echo "    Binary source: ${BINARY_PATH}"
echo "    Destination:   ${BUNDLE_DIR}"

if [ ! -f "${BINARY_PATH}" ]; then
  echo "Error: Binary not found at ${BINARY_PATH}. Build renderd-host first." >&2
  exit 1
fi

INFO_PLIST="${WORKSPACE_ROOT}/crates/renderd-host/Info.plist"
if [ ! -f "${INFO_PLIST}" ]; then
  echo "Error: Info.plist not found at ${INFO_PLIST}." >&2
  exit 1
fi

ENTITLEMENTS="${WORKSPACE_ROOT}/crates/renderd-host/entitlements.plist"

mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

cp "${BINARY_PATH}" "${MACOS_DIR}/renderd-host"
chmod +x "${MACOS_DIR}/renderd-host"

cp "${INFO_PLIST}" "${CONTENTS_DIR}/Info.plist"

if [ -f "${ENTITLEMENTS}" ]; then
  cp "${ENTITLEMENTS}" "${CONTENTS_DIR}/entitlements.plist"
fi

echo "==> renderd-host.app bundle assembled successfully"
