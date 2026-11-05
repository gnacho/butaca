#!/bin/sh
# Build the hostsim application for desktop browsers using the pinned user-local Emscripten SDK.
# This produces JS/Wasm link artifacts, not a packaged or browser-verified application shell.
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
CARGO_INCREMENTAL=0 \
CC=emcc \
CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_LINKER=emcc \
CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUSTFLAGS='-C link-arg=--use-port=sdl2 -C link-arg=-sUSE_SDL_TTF=2 -C link-arg=-sMIN_WEBGL_VERSION=1 -C link-arg=-sMAX_WEBGL_VERSION=1' \
    cargo +nightly build \
        --bin butaca-sim \
        --no-default-features \
        --features hostsim \
        --target wasm32-unknown-emscripten

printf 'Linked browser artifacts: %s\n' "$repo/rust-modules/target/wasm32-unknown-emscripten/debug/butaca-sim.js"
printf 'and %s/butaca_sim.wasm\n' "$repo/rust-modules/target/wasm32-unknown-emscripten/debug"
