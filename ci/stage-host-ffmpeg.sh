#!/bin/sh
# Stage the HOST FFmpeg beside the app assets so the host tests can bind it.
#
# WHY A SEPARATE DIRECTORY, and not `pkg/` itself. The host build has the SAME basenames as the
# shipped libraries (`libavformat-plx.so.63`), because both come from one configure line with one
# component list and only the machine differs. `pkg/lib*-plx.so.*` is exactly what `FFMPEG_STAGED`
# stages for a television and what `APP_FILES` puts in an `.ipk`, so a host copy dropped there
# would be a 64-bit library in a package for a 32-bit device, and nothing in the build would
# complain: the glob that ships them does not read ELF headers. A subdirectory is never in
# `APP_FILES`, so the two builds cannot be confused by name or by glob.
#
# WHY NO NAME REWRITING. On this platform each library carries a plain SONAME and the app opens
# the three in dependency order with RTLD_GLOBAL, so a reference to `libavutil-plx.so.61` resolves
# against the object already opened by name. The macOS stage script had to rewrite install names
# because dyld resolves by install name instead; that script retired with the platform.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PREFIX="$ROOT/vendor/ffmpeg-prefix-host"
OUT="$ROOT/pkg/host-ffmpeg"
LIBS='libavutil-plx libavcodec-plx libavformat-plx libswscale-plx'

mkdir -p "$OUT"
for name in $LIBS; do
  # `-L` because configure links the major SONAME to the real file; copy the bytes, not the link.
  src=$(ls "$PREFIX/lib/$name".so.* 2>/dev/null | grep -v '\.link\.' | head -1)
  [ -n "$src" ] || {
    echo "no host $name in $PREFIX — run 'HOST=1 ci/build-ffmpeg.sh'" >&2
    exit 1
  }
  soname=$(LC_ALL=C readelf -d "$src" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')
  [ -n "$soname" ] || {
    echo "no SONAME in $src — refusing to guess the name the app will ask for" >&2
    exit 1
  }
  cp -L "$src" "$OUT/$soname"
  chmod u+w "$OUT/$soname"
  printf '  %-28s staged\n' "$soname"
done
