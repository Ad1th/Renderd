#!/usr/bin/env bash
# Remakes the two halftone figures in site/static/img.
#   1. three.js draws the scene (scene.html) in headless Chromium (snap.js).
#   2. halftone.py turns the render into 45 degree carrot dots.
#   3. twocolour.py crops it and saves a 2-colour PNG.
# To use a real photo instead, start at step 2 with the photo as input.
# Needs: node, python3 with numpy and pillow, and a Chromium that Playwright can find.
set -euo pipefail
cd "$(dirname "$0")"
npm install --no-audit --no-fund >/dev/null
python3 -m http.server 8765 >/dev/null 2>&1 & server=$!
trap 'kill $server' EXIT
sleep 1
out=../static/img
node snap.js "http://127.0.0.1:8765/scene.html?w=2700&h=1700&cx=-10&cy=46&cz=150&tx=18&ty=15&tz=-4&fov=24" /tmp/desk.png 2700 1700
node snap.js "http://127.0.0.1:8765/scene.html?frame=1" /tmp/frame.png 1920 1080
python3 halftone.py /tmp/desk.png /tmp/desk-h.png 8 1800 1.1 FF7544 0.1 0.94
python3 halftone.py /tmp/frame.png /tmp/frame-h.png 9 960 1.1 FF7544 0.08 0.97
python3 twocolour.py /tmp/desk-h.png $out/desk-halftone.png 120,60,1680,1020
python3 twocolour.py /tmp/frame-h.png $out/frame-halftone.png
