#!/bin/sh
# Compile athp source files one by one with the kernel's flags, for
# working through the port: tools/athp-cc.sh [file.c ...]
HERE=$(cd "$(dirname "$0")/.." && pwd)
DFLY_SRC=$HERE/work/dragonfly
S=$DFLY_SRC/sys
OBJ=$HERE/work/obj
cp -R "$HERE/sys/dev/netif/athp/." "$S/dev/netif/athp/"
eval "$(sed -n '/^CFLAGS="--target/,/EXTRA_CFLAGS"$/p' "$HERE/build.sh")"
MI_CFLAGS="$CFLAGS -w"
eval "$(sed -n '/^ATHP_CFLAGS=/,/-DATHP_DFLY"$/p' "$HERE/build.sh")"
cd "$S/dev/netif/athp" || exit 1
for f in "$@"; do
	clang $ATHP_CFLAGS -ferror-limit=0 -c "$f" -o /dev/null 2>&1 | grep -E "error:" |
	    sed "s#^$S/dev/netif/athp/##" | head -${N:-8}
done
