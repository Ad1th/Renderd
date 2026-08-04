#!/usr/bin/env bash
# Sync and manage Renderd GitHub Milestones using GitHub CLI (gh)

set -euo pipefail

if ! command -v gh &> /dev/null; then
  echo "Error: gh CLI is required but not installed." >&2
  exit 1
fi

echo "Syncing Renderd Milestones..."

MILESTONES=(
  "Milestone 1: Repository Bootstrap & Infrastructure|v0.1.0-bootstrap|Workspace layout, cargo config, Clippy/deny policy, initial CI workflows."
  "Milestone 2: Foundation Layer|v0.2.0-foundation|Protobuf schema, renderd-proto crate, and renderd-config crate."
  "Milestone 3: Core Data Structures & Utilities|v0.3.0-primitives|Fragment header codec, reassembly buffer, clock estimator, and ABR engine."
  "Milestone 4: macOS Host Capture Engine|v0.4.0-capture|ScreenCaptureKit zero-copy capture, VideoToolbox hardware HEVC encoder FFI."
  "Milestone 5: Networking & Transport|v0.5.0-transport|QUIC socket transport, Noise protocol encryption, mDNS peer discovery."
  "Milestone 6: Windows Viewer Engine|v0.6.0-viewer|Direct3D12 presentation swapchain, MediaFoundation hardware decoder."
  "Milestone 7: Integration & Daemons|v0.7.0-daemons|renderd-host daemon and renderd-viewer client binary assembly."
  "Milestone 8: Benchmarks & Tooling|v0.8.0-benchmarks|End-to-end latency benchmark suite and performance monitoring."
  "Milestone 9: Documentation & Quality|v0.9.0-quality|Comprehensive API docs, security audits, and deployment instructions."
  "Milestone 10: Pre-Release Audit & Release|v1.0.0|Final security audit, performance verification, and initial production release."
)

for entry in "${MILESTONES[@]}"; do
  IFS='|' read -r title tag description <<< "$entry"
  echo "Processing: $title ($tag)"

  # Check if milestone exists
  if gh api "repos/{owner}/{repo}/milestones" --jq ".[] | select(.title==\"$title\") | .number" | grep -q .; then
    echo "  -> Milestone '$title' already exists."
  else
    echo "  -> Creating milestone '$title'..."
    gh api "repos/{owner}/{repo}/milestones" \
      -f title="$title" \
      -f description="$description ($tag)" \
      -f state="open" > /dev/null
  fi
done

echo "Milestone synchronization complete."
