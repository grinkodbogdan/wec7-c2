#!/bin/sh
# compat/android/atl/build.sh OUT - Android Translation Layer on Flare.
#
# Builds ATL (https://gitlab.com/android_translation_layer, GPL-3.0) for
# Flare without GTK:
#   * the Java framework (src/api-impl), compiled with javac and dexed with
#     d8 into api-impl.jar, plus ATL's prebuilt framework-res.apk;
#   * libtranslation_layer_main.so: ATL's toolkit-independent JNI sources
#     (assets, resources, XML, SQLite, Looper, logging, ...) and Flare's
#     native backend in flare/ in place of the GTK one: views, widgets,
#     canvas, text, bitmaps and an Arcan window. Natives without a backend
#     yet get logging stubs (gen_stubs.py);
#   * atl-run, the launcher;
# against a sysroot of Alpine edge packages (alpine.lock, the same release
# as ART in compat/android/art), and stages them as /opt/atl in OUT/stage.
#
# Needs: clang, lld, llvm-readelf/llvm-strip, a JDK (javac, javap, jar),
# git, curl, python3; D8 (Android build-tools' d8), ARCAN_ROOT (the
# Arcan install from compat/arcan/build.sh) and ART_STAGE (the staged
# /opt/art from compat/android/art/build.sh) in the environment.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
: "${D8:?set D8 to Android build-tools d8}"
: "${ARCAN_ROOT:?set ARCAN_ROOT to the Arcan install root}"
: "${ART_STAGE:?set ART_STAGE to the staged ART (compat/android/art/build.sh OUT/stage)}"
jobs=$(nproc)

ATL_COMMIT=22a96b28ee007d610f610c6f37aa81509195e288
MAVEN=https://repo1.maven.org/maven2
ASM="org/ow2/asm/asm/9.7/asm-9.7.jar adf46d5e34940bdf148ecdd26a9ee8eea94496a72034ff7141066b3eea5c4e9d"
ASM_TREE="org/ow2/asm/asm-tree/9.7/asm-tree-9.7.jar 62f4b3bc436045c1acb5c3ba2d8ec556ec3369093d7f5d06c747eb04b56d52b1"
GETOPT="gnu/getopt/java-getopt/1.0.13/java-getopt-1.0.13.jar f09dc6369ffc4fe4362cbd49b6ac6c4a7471375ac5d884c71b0a625a889c38c4"

cache="$out/cache"
sys="$out/sysroot"
b="$out/build"
mkdir -p "$cache" "$b"

echo "atl/build.sh: sysroot (Alpine edge packages)"
rm -rf "$sys"
ALPINE_RELEASE=edge ALPINE_REPOS="main community testing" \
    python3 "$here/../../arcan/alpine.py" "$cache" "$sys" "$here/alpine.lock" \
    musl-dev gcc libstdc++-dev linux-headers art_standalone-dev glib-dev cairo-dev pango-dev \
    freetype-dev fontconfig-dev libandroidfw-dev sqlite-dev libpng-dev font-dejavu

echo "atl/build.sh: ATL $ATL_COMMIT"
src="$out/src"
if [ "$(git -C "$src" rev-parse HEAD 2>/dev/null)" != "$ATL_COMMIT" ]; then
    rm -rf "$src"
    git init -q "$src"
    git -C "$src" fetch -q --depth 1 https://gitlab.com/android_translation_layer/android_translation_layer.git "$ATL_COMMIT"
    git -C "$src" -c advice.detachedHead=false checkout -q FETCH_HEAD
fi
git -C "$src" checkout -q -- .
for p in "$here"/patches/*.patch; do
    [ -e "$p" ] && git -C "$src" apply "$p"
done

echo "atl/build.sh: Java framework"
for dep in "$ASM" "$ASM_TREE" "$GETOPT"; do
    set -- $dep
    f="$cache/$(basename "$1")"
    [ -f "$f" ] || curl -fsSL --retry 3 -o "$f" "$MAVEN/$1"
    echo "$2  $f" | sha256sum -c - >/dev/null
done
bcp="$sys/usr/lib/java/core-all_classes.jar:$sys/usr/lib/java/core-junit_classes.jar"
rm -rf "$b/classes" "$b/gen" "$b/dex"
mkdir -p "$b/classes" "$b/gen" "$b/dex"
(cd "$src/src/api-impl" && find . -name '*.java' | sort) > "$b/sources.txt"
(cd "$src/src/api-impl" && javac -nowarn -source 8 -target 8 -encoding UTF-8 -parameters -Xlint:none \
    -bootclasspath "$bcp" -d "$b/classes" @"$b/sources.txt") 2>&1 | grep -v "^warning\|^Note:\|^[0-9]* warning" || true
(cd "$b/classes" && jar cf "$b/hax.jar" .)
"$D8" --min-api 26 --output "$b/dex" "$b/hax.jar" 2>&1 | grep -v "^Warning" || true
(cd "$b/dex" && rm -f "$b/api-impl.jar" && jar cf "$b/api-impl.jar" classes*.dex)

echo "atl/build.sh: JNI upcalls and stubs"
mvn="$cache/asm-9.7.jar:$cache/asm-tree-9.7.jar:$cache/java-getopt-1.0.13.jar"
javac -nowarn -cp "$mvn" -d "$b/gen" "$src/tools/jni_upcall_gen/src/JNIUpcallWrapperGenerator.java"
java -cp "$b/gen:$mvn" JNIUpcallWrapperGenerator --jar-file "$b/hax.jar" -d "$b/upcalls.h" \
    --classpath "$bcp" --short-class-names "$src/extern_upcalls.txt" >/dev/null
python3 "$here/gen_stubs.py" "$b/hax.jar" "$b/stubs.c"

echo "atl/build.sh: libtranslation_layer_main.so, atl-run"
jni=$(dirname "$(dirname "$(readlink -f "$(command -v javac)")")")/include
shim="$b/shim"
mkdir -p "$shim/gtk"
# util.h includes GTK only for helpers the Flare backend does not use
cat > "$shim/gtk/gtk.h" <<'EOF'
#include <glib.h>
#include <glib-object.h>
#include <gio/gio.h>
typedef struct _GtkWidget GtkWidget;
typedef struct _GtkLabel GtkLabel;
EOF
# util.c without its GTK helpers (atl_ensure_widget_snapshotability, atl_safe_gtk_*)
awk '/^\/\* Calling these functions while snapshotting/ {skip=1} /^GVariant \*intent_serialize/ {skip=0} !skip' \
    "$src/src/api-impl-jni/util.c" > "$b/util.c"
pc() { PKG_CONFIG_LIBDIR="$sys/usr/lib/pkgconfig:$sys/usr/share/pkgconfig" PKG_CONFIG_SYSROOT_DIR="$sys" pkg-config "$@"; }
CC="clang --target=aarch64-alpine-linux-musl --sysroot=$sys --gcc-toolchain=$sys/usr -fuse-ld=lld"
CFLAGS="-O2 -g0 -fPIC -Qunused-arguments -Wno-macro-redefined -D_GNU_SOURCE -D_LARGEFILE64_SOURCE -Wno-deprecated-declarations -Wno-int-conversion
    -Wno-incompatible-pointer-types -Wno-implicit-function-declaration
    -I$shim -I$b -I$src/src/api-impl-jni -I$src/src -I$src -I$jni -I$jni/linux -I$sys/usr/include/androidfw
    -I$ARCAN_ROOT/usr/include/arcan/shmif $(pc --cflags glib-2.0 gio-2.0 cairo pangocairo sqlite3)"
LIBS="$(pc --libs glib-2.0 gio-2.0 cairo pangocairo sqlite3) -L$sys/usr/lib/art -landroidfw -lutils -llog"
jsrc="$src/src/api-impl-jni"
reuse="AssetInputStream.c android_content_res_AssetManager.c android_os_Environment.c android_os_MemoryFile.c
    android_os_MessageQueue.c android_os_SystemClock.c android_util_Log.c content/android_content_ContentResolver.c
    content/android_content_res_XmlBlock.c database/android_database_SQLiteCommon.c
    database/android_database_SQLiteConnection.c database/android_database_SQLiteGlobal.c handle_cache.c
    net/android_net_ConnectivityManager.c os/android_atl_GVariantBuilderParcel.c os/android_atl_GVariantIterParcel.c
    os/android_os_Process.c"
rm -rf "$b/obj" && mkdir -p "$b/obj"
objs=""
for f in $reuse; do
    o="$b/obj/$(echo "$f" | tr / _).o"
    $CC $CFLAGS -c "$jsrc/$f" -o "$o" &
    objs="$objs $o"
done
for f in "$b/util.c" "$src/src/libandroid/looper.c" "$b/stubs.c" "$here"/flare/gfx.c "$here"/flare/views.c "$here"/flare/window.c; do
    o="$b/obj/$(basename "$f" .c).o"
    $CC $CFLAGS -Werror=implicit-function-declaration -I"$here/flare" -c "$f" -o "$o" &
    objs="$objs $o"
done
wait
for o in $objs; do [ -f "$o" ] || { echo "atl/build.sh: compiling failed ($o)"; exit 1; }; done
$CC -shared -o "$b/libtranslation_layer_main.so" $objs $LIBS -L"$ARCAN_ROOT/usr/lib" -larcan_shmif -Wl,-soname,libtranslation_layer_main.so
$CC $CFLAGS -I"$here/flare" -DATL_PREFIX='"/opt/atl"' "$here/flare/main.c" -o "$b/atl-run" \
    -L"$b" -ltranslation_layer_main -L"$sys/usr/lib/art" -lart $LIBS -L"$ARCAN_ROOT/usr/lib" -larcan_shmif

echo "atl/build.sh: staging"
python3 "$here/stage.py" "$sys" "$b" "$src" "$out/stage" "$ARCAN_ROOT" "$ART_STAGE/opt/art/lib"
install -D -m 755 "$here/atl-run" "$out/stage/usr/bin/atl-run"
echo "atl/build.sh: done ($(du -sh "$out/stage" | cut -f1) staged)"
