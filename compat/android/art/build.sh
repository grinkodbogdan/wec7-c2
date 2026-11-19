#!/bin/sh
# compat/android/art/build.sh OUT - ART, the Android runtime, for Flare.
#
# Takes the prebuilt aarch64 ART of Alpine edge (art_standalone, the
# standalone ART that Android Translation Layer uses; Apache-2.0), with the
# exact packages in alpine.lock, and stages it as the private prefix
# /opt/art (own musl loader and libraries) in OUT/stage, plus art-run.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
rm -rf "$out/root"
ALPINE_RELEASE=edge ALPINE_REPOS="main community testing" \
    python3 "$here/../../arcan/alpine.py" "$out/cache" "$out/root" "$here/alpine.lock" art_standalone bionic_translation
python3 "$here/stage.py" "$out/root" "$out/stage"
install -D -m 755 "$here/art-run" "$out/stage/usr/bin/art-run"
echo "art/build.sh: done ($(du -sh "$out/stage" | cut -f1) staged)"
