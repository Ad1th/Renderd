#!/usr/bin/env bash
# Sync canonical GitHub labels from .github/labels.yml using GitHub CLI (gh)

set -euo pipefail

LABELS_FILE=".github/labels.yml"

if ! command -v gh &> /dev/null; then
  echo "Error: gh CLI is required but not installed." >&2
  exit 1
fi

if [ ! -f "$LABELS_FILE" ]; then
  echo "Error: Labels file '$LABELS_FILE' not found." >&2
  exit 1
fi

echo "Synchronizing GitHub labels from $LABELS_FILE..."

# Use gh label import if available, or parse YAML and create/update via gh api
if gh label import --help &> /dev/null; then
  gh label import "$LABELS_FILE" --force
  echo "Labels synchronized successfully using gh label import."
else
  echo "Importing labels via gh API..."
  # Fallback using python or awk to parse simple YAML items if gh label import is unavailable
  python3 -c "
import yaml, subprocess
with open('$LABELS_FILE') as f:
    labels = yaml.safe_load(f)
for l in labels:
    cmd = ['gh', 'label', 'create', l['name'], '--color', l['color'], '--description', l.get('description', ''), '--force']
    subprocess.run(cmd, check=True)
"
  echo "Labels synchronized successfully."
fi
