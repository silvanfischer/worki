#!/usr/bin/env bash
# Assembles the website into a folder: the pages in site/, the README screenshots and the app
# icon. GitHub Pages publishes the result (.github/workflows/pages.yml).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/target/site}"

rm -rf "$out"
mkdir -p "$out/screenshots"
cp "$root"/site/* "$out/"
cp "$root"/docs/screenshots/*.png "$out/screenshots/"
cp "$root/src-tauri/icons/icon.png" "$out/icon.png"
cp "$root/src-tauri/icons/32x32.png" "$out/favicon.png"
echo "Website assembled in $out"
