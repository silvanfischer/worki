#!/bin/sh
# Renders the README screenshots: the real UI with demo data (ui/demo.html mocks the Tauri side),
# captured by headless Chrome at 2x into docs/screenshots.
set -eu
cd "$(dirname "$0")/.."

CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
PORT=1431
OUT=docs/screenshots

node_modules/.bin/vite --port "$PORT" --strictPort >/dev/null 2>&1 &
VITE=$!
trap 'kill "$VITE" 2>/dev/null' EXIT
until curl -sf "http://localhost:$PORT/demo.html" >/dev/null; do sleep 0.3; done

PROFILE=$(mktemp -d)
for scene in main chat terminal branch isolation settings; do
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-dark-mode \
    --force-device-scale-factor=2 --window-size=1600,1000 --virtual-time-budget=5000 \
    --user-data-dir="$PROFILE" --screenshot="$OUT/$scene.png" \
    "http://localhost:$PORT/demo.html?scene=$scene" 2>/dev/null
  echo "$OUT/$scene.png"
done
rm -rf "$PROFILE"
