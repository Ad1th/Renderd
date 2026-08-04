#!/usr/bin/env bash
# Prepare a new release: run checks, bump version, commit release, and tag

set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <version>" >&2
  echo "Example: $0 0.3.0" >&2
  exit 1
fi

VERSION="${1#v}"
TAG_NAME="v${VERSION}"

echo "=========================================="
echo " Preparing Renderd Release ${TAG_NAME}"
echo "=========================================="

# 1. Verify working directory is clean
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: Working directory has uncommitted changes. Please commit or stash them first." >&2
  git status --short
  exit 1
fi

# 2. Verify current branch is main or a release branch
BRANCH=$(git rev-parse --abbrev-ref HEAD)
echo "Current branch: $BRANCH"

# 3. Run validation checks
echo "Running quality verification checks..."
echo "  -> cargo fmt --check"
cargo fmt --check

echo "  -> cargo clippy --workspace --all-targets"
cargo clippy --workspace --all-targets -- -D warnings

echo "  -> cargo nextest run --workspace"
if command -v cargo-nextest &> /dev/null; then
  cargo nextest run --workspace
else
  cargo test --workspace
fi

echo "  -> cargo deny check"
if command -v cargo-deny &> /dev/null; then
  cargo deny check
fi

# 4. Bump version
echo "Bumping version in Cargo.toml..."
bash scripts/bump-version.sh "$VERSION"

# 5. Create Release Commit & Tag
echo "Creating release commit..."
git add Cargo.toml Cargo.lock
git commit -m "chore(release): prepare ${TAG_NAME}"

echo "Creating signed tag ${TAG_NAME}..."
git tag -a "${TAG_NAME}" -m "Release ${TAG_NAME}"

echo "=========================================="
echo " Release ${TAG_NAME} prepared successfully!"
echo " To publish the release, run:"
echo "   git push origin $BRANCH"
echo "   git push origin ${TAG_NAME}"
echo "=========================================="
