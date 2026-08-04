#!/usr/bin/env bash
# Bump workspace version across Cargo.toml, Cargo.lock, and documentation

set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <new_version>" >&2
  echo "Example: $0 0.3.0" >&2
  exit 1
fi

NEW_VERSION="$1"
# Strip leading 'v' if provided
NEW_VERSION="${NEW_VERSION#v}"

echo "Bumping Renderd workspace version to $NEW_VERSION..."

CARGO_TOML="Cargo.toml"

if [ ! -f "$CARGO_TOML" ]; then
  echo "Error: Cargo.toml not found in current directory." >&2
  exit 1
fi

# Update version in [workspace.package]
sed -i '' -e "s/^version      = \".*\"/version      = \"$NEW_VERSION\"/" "$CARGO_TOML" || \
sed -i -e "s/^version      = \".*\"/version      = \"$NEW_VERSION\"/" "$CARGO_TOML"

echo "Updated $CARGO_TOML version field to $NEW_VERSION."

# Update Cargo.lock via cargo check
echo "Updating Cargo.lock..."
cargo check --workspace > /dev/null

echo "Version bump complete. Current Cargo workspace package version:"
grep '^version      =' "$CARGO_TOML"
