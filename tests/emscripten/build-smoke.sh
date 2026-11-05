#!/bin/sh
# Build the SDL2/GLES2 smoke app as a WebGL 1 page using the pinned user-local Emscripten SDK.
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
sdk_dir=${EMSDK_DIR:-${XDG_DATA_HOME:-"$HOME/.local/share"}/emsdk}
if [ ! -f "$sdk_dir/emsdk_env.sh" ]; then
    echo "Emscripten SDK not found at $sdk_dir; run tools/setup-emsdk.sh first" >&2
    exit 2
fi

# shellcheck disable=SC1090
. "$sdk_dir/emsdk_env.sh" >/dev/null
out="$repo/rust-modules/target/emscripten-smoke"
mkdir -p "$out"
emcc "$repo/tests/emscripten/sdl_webgl_smoke.c" \
    --use-port=sdl2 \
    -sMIN_WEBGL_VERSION=1 -sMAX_WEBGL_VERSION=1 \
    -sASSERTIONS=2 \
    -O2 \
    -o "$out/index.html"
printf 'Built %s\n' "$out/index.html"
