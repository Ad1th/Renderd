#!/usr/bin/env bash
# Writes redirect pages for the old GitHub Pages site (ad1th.github.io/Renderd),
# so its links and search ranking move to renderd.xyz. GitHub Pages cannot send
# a 301, so each page has a canonical link and a meta refresh.
# Usage: site/scripts/pages-redirects.sh <out-dir>
set -euo pipefail
out=${1:?out dir}
mkdir -p "$out"
site=https://renderd.xyz
repo=https://github.com/Ad1th/Renderd

page() { # page <file> <new url>
  cat > "$out/$1" <<HTML
<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Renderd has moved to renderd.xyz</title>
<link rel="canonical" href="$2">
<meta http-equiv="refresh" content="0; url=$2">
</head><body><p>Renderd has moved. Go to <a href="$2">$2</a>.</p></body></html>
HTML
}

page index.html "$site/"
page 404.html "$site/"
for p in overview features platforms faq; do page "$p.html" "$site/"; done
page architecture.html "$site/#how"
page docs.html "$site/download/#first-run"
page contributing.html "$repo/blob/main/CONTRIBUTING.md"
page releases.html "$repo/releases"
page roadmap.html "$repo#project-feature-roadmap"
echo "wrote $(ls "$out" | wc -l) redirect pages to $out"
