#!/usr/bin/env bash
# Generate or update CHANGELOG.md using git-cliff or conventional commit parsing

set -euo pipefail

CHANGELOG_FILE="CHANGELOG.md"

if command -v git-cliff &> /dev/null; then
  echo "Generating changelog using git-cliff..."
  git-cliff --output "$CHANGELOG_FILE"
  echo "CHANGELOG.md successfully updated with git-cliff."
else
  echo "git-cliff not found; performing git conventional commit log extraction..."

  LATEST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")

  if [ -n "$LATEST_TAG" ]; then
    echo "Extracting conventional commits since tag $LATEST_TAG:"
    RANGE="$LATEST_TAG..HEAD"
  else
    echo "Extracting conventional commits across entire history:"
    RANGE="HEAD"
  fi

  echo ""
  echo "### Added"
  git log "$RANGE" --oneline --grep="^feat" | sed 's/^[a-f0-9]* /- /' || true

  echo ""
  echo "### Fixed"
  git log "$RANGE" --oneline --grep="^fix" | sed 's/^[a-f0-9]* /- /' || true

  echo ""
  echo "### Performance"
  git log "$RANGE" --oneline --grep="^perf" | sed 's/^[a-f0-9]* /- /' || true

  echo ""
  echo "### Security"
  git log "$RANGE" --oneline --grep="^security" | sed 's/^[a-f0-9]* /- /' || true

  echo ""
  echo "### Changed / Maintenance"
  git log "$RANGE" --oneline --grep="^\(refactor\|chore\|ci\|docs\)" | sed 's/^[a-f0-9]* /- /' || true
fi
