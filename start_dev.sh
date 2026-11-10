#!/bin/sh
# Compiles worki and starts it in development mode, with hot reload for the UI.
set -eu
cd "$(dirname "$0")"

export COREPACK_ENABLE_DOWNLOAD_PROMPT=0

if [ ! -d node_modules ] || [ pnpm-lock.yaml -nt node_modules/.modules.yaml ]; then
  pnpm install --frozen-lockfile
fi

exec pnpm tauri dev "$@"
