#!/bin/sh
# Build the hostsim application for desktop browsers using the pinned user-local Emscripten SDK.
# This produces the Emscripten default HTML shell plus JS/Wasm and preloaded font/logo assets;
# it is a development browser build, not a production package.
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sdk_dir=${EMSDK_DIR:-${XDG_DATA_HOME:-"$HOME/.local/share"}/emsdk}
if [ ! -f "$sdk_dir/emsdk_env.sh" ]; then
    echo "Emscripten SDK not found at $sdk_dir; run tools/setup-emsdk.sh first" >&2
    exit 2
fi

# emsdk_env.sh uses its working directory to find the SDK under POSIX /bin/sh.
cd "$sdk_dir"
# shellcheck disable=SC1091
. ./emsdk_env.sh >/dev/null

# Build ports separately so SDL_ttf's nested FreeType/Harfbuzz builds do not contend for
# Emscripten's cache lock during the Rust link.
embuilder build sdl2 sdl2_ttf

cd "$repo/rust-modules"
target_dir="$repo/rust-modules/target"
html_out="$target_dir/wasm32-unknown-emscripten/debug/butaca-sim.html"
rust_features=${BUTACA_EMSCRIPTEN_FEATURES:-hostsim}
CARGO_INCREMENTAL=0 \
CC=emcc \
CARGO_TARGET_DIR="$target_dir" \
CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_LINKER=emcc \
CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUSTFLAGS="-C link-arg=--use-port=sdl2 -C link-arg=-sUSE_SDL_TTF=2 -C link-arg=-sMIN_WEBGL_VERSION=1 -C link-arg=-sMAX_WEBGL_VERSION=1 -C link-arg=-sASYNCIFY=1 -C link-arg=--pre-js -C link-arg=$repo/tests/emscripten/browser-pre.js -C link-arg=--preload-file=$repo/pkg/appfont.ttf@/app/appfont.ttf -C link-arg=--preload-file=$repo/pkg/appfont-bold.ttf@/app/appfont-bold.ttf -C link-arg=--preload-file=$repo/pkg/appfont-cjk.ttf@/app/appfont-cjk.ttf -C link-arg=--preload-file=$repo/pkg/butaca-name.png@/app/butaca-name.png -C link-arg=--oformat=html -C link-arg=-o -C link-arg=$html_out" \
    cargo +nightly build \
        --bin butaca-sim \
        --no-default-features \
        --features "$rust_features" \
        --target wasm32-unknown-emscripten

printf 'Browser HTML shell: %s\n' "$html_out"
printf 'JavaScript glue: %s/butaca-sim.js\n' "$target_dir/wasm32-unknown-emscripten/debug"
printf 'Wasm module and assets: %s/butaca-sim.wasm and .data\n' "$target_dir/wasm32-unknown-emscripten/debug"
