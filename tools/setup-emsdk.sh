#!/bin/sh
# Install the repository-pinned official Emscripten SDK under the user's data directory.
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
pin="$repo/tools/emsdk-pin"
read -r version revision < "$pin"
if [ -z "${version:-}" ] || [ -z "${revision:-}" ]; then
    echo "invalid Emscripten SDK pin: $pin" >&2
    exit 2
fi

sdk_dir=${EMSDK_DIR:-${XDG_DATA_HOME:-"$HOME/.local/share"}/emsdk}
if [ ! -d "$sdk_dir/.git" ]; then
    if [ -e "$sdk_dir" ]; then
        echo "SDK path exists but is not an emsdk checkout: $sdk_dir (set EMSDK_DIR to another path)" >&2
        exit 2
    fi
    mkdir -p "$(dirname -- "$sdk_dir")"
    git init "$sdk_dir"
    git -C "$sdk_dir" remote add origin https://github.com/emscripten-core/emsdk.git
    git -C "$sdk_dir" fetch --depth 1 origin "$revision"
    git -C "$sdk_dir" checkout --detach FETCH_HEAD
else
    actual=$(git -C "$sdk_dir" rev-parse HEAD)
    if [ "$actual" != "$revision" ]; then
        echo "emsdk checkout at $sdk_dir is $actual, expected $revision (set EMSDK_DIR to another path)" >&2
        exit 2
    fi
fi

"$sdk_dir/emsdk" install "$version"
"$sdk_dir/emsdk" activate "$version"
printf 'Installed Emscripten %s at %s\n' "$version" "$sdk_dir"
printf 'For this shell, run: . "%s/emsdk_env.sh"\n' "$sdk_dir"
