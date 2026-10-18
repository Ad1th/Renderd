#!/usr/bin/env bash
# Builds renderd.xyz into site/public. Vercel runs this (see vercel.json);
# it works the same on a Linux laptop or in CI.
#   1. Get Zola 0.23.6 (static musl build, checked against a pinned SHA-256).
#   2. Write data/release.json from the latest GitHub release.
#   3. zola build.
#   4. Add rustdoc at /api/. If that fails, the site still ships without it.
# Set RENDERD_SKIP_API=1 to skip step 4. RENDERD_BASE_URL sets the site address.
set -euo pipefail
cd "$(dirname "$0")"

ZOLA_VERSION=0.23.6
ZOLA_SHA256=7d77a4220a4f699cba2fc0e0c7859f780e887e269159ad4fab90e11ea16c906f
tools="${TMPDIR:-/tmp}/renderd-site-tools"
zola="$tools/zola-$ZOLA_VERSION"

if [ ! -x "$zola" ]; then
  mkdir -p "$tools"
  curl -fsSL --retry 3 -o "$tools/zola.tgz" \
    "https://github.com/getzola/zola/releases/download/v$ZOLA_VERSION/zola-v$ZOLA_VERSION-x86_64-unknown-linux-musl.tar.gz"
  echo "$ZOLA_SHA256  $tools/zola.tgz" | sha256sum -c -
  tar -xzf "$tools/zola.tgz" -C "$tools" zola
  mv "$tools/zola" "$zola"
fi
"$zola" --version

node scripts/release.mjs

# Zola writes absolute links from base_url. Production uses renderd.xyz (config.toml);
# a Vercel preview uses its own address, or its CSS and fonts would load from
# production. RENDERD_BASE_URL overrides both, for example while DNS is not ready.
base_url="${RENDERD_BASE_URL:-}"
if [ -z "$base_url" ] && [ "${VERCEL_ENV:-}" = "preview" ] && [ -n "${VERCEL_URL:-}" ]; then
  base_url="https://$VERCEL_URL"
fi
if [ -n "$base_url" ]; then
  echo "base_url: $base_url"
  "$zola" build --base-url "$base_url"
else
  "$zola" build
fi

if [ "${RENDERD_SKIP_API:-0}" != "1" ]; then
  # set -e does not work inside an if condition, so each step is chained with &&.
  if (
    { command -v cargo >/dev/null ||
      curl -fsSL --retry 3 https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path; } &&
    export PATH="$HOME/.cargo/bin:$PATH" &&
    # Only the docs: skip the macOS and Windows targets in rust-toolchain.toml.
    cd .. && RUSTUP_TOOLCHAIN=stable cargo doc --no-deps --workspace \
      --exclude renderd-host --exclude renderd-viewer --exclude renderd-vt-sys --exclude renderd-sc-sys
  ); then
    mkdir -p public/api
    cp -r ../target/doc/. public/api/
    echo "rustdoc: added at /api/"
  else
    echo "rustdoc: build failed, the site ships without /api/" >&2
  fi
fi
