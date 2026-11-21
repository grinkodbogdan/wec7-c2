#!/bin/sh
# compat/arcan/build.sh OUT - Arcan 0.6 for Flare's Linux personality.
#
# Builds, for aarch64 Linux (musl), dynamically linked:
#   * Mesa 25.1 with llvmpipe (shaders compiled to native code by LLVM,
#     linked statically; each frame split across all CPUs) and softpipe:
#     EGL, GBM and GLES2, rendering into KMS dumb buffers (kms_swrast);
#   * Arcan 0.6.3.4 with the egl-dri video platform and the GLES2 backend;
#   * kmstest, a GBM/EGL/KMS self-check;
# against a sysroot of Alpine packages (compat/arcan/alpine.lock), then
# stages them with their libraries and the DejaVu fonts in OUT/stage, the
# overlay `cargo xtask build` puts into the root filesystem.
#
# Needs: clang, lld, llvm-readelf/llvm-strip, meson, ninja, cmake, bison,
# flex, pkg-config, git, curl, and python3 with mako and pyyaml.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
jobs=$(nproc)

MESA_VERSION=25.1.9
MESA_SHA256=412df33a1bb3c785ed698555a3972118a37c458e7accf6ae53f4bb87b3db454a
ARCAN_TAG=0.6.3.4
ARCAN_COMMIT=697be3bb41678ae5fe58a45c68c10a46eb326ad4

cache="$out/cache"
sys="$out/sysroot"
mkdir -p "$cache" "$out/src"

echo "arcan/build.sh: sysroot (Alpine packages)"
rm -rf "$sys"
python3 "$here/alpine.py" "$cache" "$sys" "$here/alpine.lock" \
    musl-dev libstdc++-dev gcc linux-headers zlib-dev expat-dev libdrm-dev freetype-dev \
    libpng-dev sqlite-dev luajit-dev libxkbcommon-dev font-dejavu llvm20-dev llvm20-static zstd-dev

sed "s|@LLVM@|$sys/usr/lib/llvm20|" "$here/llvm-config.in" > "$out/llvm-config"
chmod +x "$out/llvm-config"
echo "arcan/build.sh: Mesa $MESA_VERSION (llvmpipe, softpipe)"
tarball="$cache/mesa-$MESA_VERSION.tar.xz"
[ -f "$tarball" ] || curl -fsSL --retry 3 -o "$tarball" "https://archive.mesa3d.org/mesa-$MESA_VERSION.tar.xz"
echo "$MESA_SHA256  $tarball" | sha256sum -c - >/dev/null
rm -rf "$out/src/mesa-$MESA_VERSION" "$out/mesa-root"
tar xJf "$tarball" -C "$out/src"
for p in "$here"/mesa-patches/*.patch; do patch -s -d "$out/src/mesa-$MESA_VERSION" -p1 < "$p"; done
cat > "$out/cross.ini" <<EOF
[binaries]
c = ['clang', '--target=aarch64-alpine-linux-musl', '--sysroot=$sys', '--gcc-toolchain=$sys/usr']
cpp = ['clang++', '--target=aarch64-alpine-linux-musl', '--sysroot=$sys', '--gcc-toolchain=$sys/usr']
ar = 'llvm-ar'
strip = 'llvm-strip'
pkg-config = 'pkg-config'
llvm-config = '$out/llvm-config'
[properties]
sys_root = '$sys'
pkg_config_libdir = '$sys/usr/lib/pkgconfig:$sys/usr/share/pkgconfig'
[built-in options]
c_link_args = ['-fuse-ld=lld']
cpp_link_args = ['-fuse-ld=lld']
[host_machine]
system = 'linux'
cpu_family = 'aarch64'
cpu = 'cortex-a53'
endian = 'little'
EOF
(
    cd "$out/src/mesa-$MESA_VERSION"
    meson setup build --cross-file "$out/cross.ini" --prefix=/usr --libdir=lib --buildtype=release \
        -Db_ndebug=true -Dgallium-drivers=llvmpipe,softpipe -Dvulkan-drivers= -Dllvm=enabled -Dshared-llvm=disabled -Dplatforms= \
        -Degl=enabled -Dgbm=enabled -Dgles1=disabled -Dgles2=enabled -Dopengl=true -Dglx=disabled \
        -Dgallium-va=disabled -Dgallium-vdpau=disabled -Dgallium-rusticl=false -Dvalgrind=disabled \
        -Dlibunwind=disabled -Dlmsensors=disabled -Dzstd=disabled -Dexpat=enabled -Dbuild-tests=false \
        -Dglvnd=disabled >"$out/mesa-setup.log"
    ninja -C build -j"$jobs" >"$out/mesa-build.log"
    DESTDIR="$out/mesa-root" ninja -C build install >/dev/null
)
cp -a "$out/mesa-root/usr/." "$sys/usr/"

echo "arcan/build.sh: Arcan $ARCAN_TAG"
src="$out/src/arcan"
rm -rf "$src" "$out/arcan-build" "$out/arcan-root"
git -c advice.detachedHead=false clone -q --depth 1 --branch "$ARCAN_TAG" https://github.com/letoram/arcan.git "$src"
[ "$(git -C "$src" rev-parse HEAD)" = "$ARCAN_COMMIT" ] || { echo "arcan tag moved"; exit 1; }
for p in "$here"/patches/*.patch; do patch -s -d "$src" -p1 < "$p"; done
cat > "$out/toolchain.cmake" <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_SYSROOT $sys)
set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)
set(CMAKE_C_COMPILER_TARGET aarch64-alpine-linux-musl)
set(CMAKE_CXX_COMPILER_TARGET aarch64-alpine-linux-musl)
set(CMAKE_C_FLAGS_INIT "--gcc-toolchain=$sys/usr")
set(CMAKE_CXX_FLAGS_INIT "--gcc-toolchain=$sys/usr")
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(ENV{PKG_CONFIG_LIBDIR} "$sys/usr/lib/pkgconfig:$sys/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "$sys")
EOF
mkdir -p "$out/arcan-build"
(
    cd "$out/arcan-build"
    cmake -G Ninja -DCMAKE_TOOLCHAIN_FILE="$out/toolchain.cmake" -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/usr -DCMAKE_C_FLAGS="-DEGL_NO_X11 -DMESA_EGL_NO_X11_HEADERS" \
        -DVIDEO_PLATFORM=egl-dri -DAGP_PLATFORM=gles2 -DAUDIO_PLATFORM=stub -DDISABLE_WAYLAND=ON \
        -DENABLE_LWA=OFF -DSTATIC_LIBUVC=OFF -DDISABLE_FSRV_ENCODE=ON -DDISABLE_FSRV_DECODE=ON \
        -DDISABLE_FSRV_NET=ON -DDISABLE_FSRV_REMOTING=ON -DDISABLE_FSRV_GAME=ON -DDISABLE_FSRV_TERMINAL=ON \
        -DOPENGL_egl_LIBRARY="$sys/usr/lib/libEGL.so" -DOPENGL_EGL_INCLUDE_DIR="$sys/usr/include" \
        -DOPENGL_opengl_LIBRARY="$sys/usr/lib/libGLESv2.so" -DOPENGL_INCLUDE_DIR="$sys/usr/include" \
        "$src/src" >"$out/arcan-setup.log" 2>&1
    ninja -j"$jobs" >"$out/arcan-build.log"
    DESTDIR="$out/arcan-root" ninja install >/dev/null
)

echo "arcan/build.sh: kmstest"
clang --target=aarch64-alpine-linux-musl --sysroot="$sys" --gcc-toolchain="$sys/usr" -fuse-ld=lld -O2 \
    -I"$sys/usr/include/libdrm" "$here/tests/kmstest.c" -o "$out/kmstest" -ldrm -lgbm -lEGL -lGLESv2

echo "arcan/build.sh: staging"
python3 "$here/stage.py" "$sys" "$out/arcan-root" "$out/mesa-root" "$out/stage" "$out/kmstest"
fonts="$out/stage/usr/share/arcan/resources/fonts"
for f in DejaVuSans-ExtraLight DejaVuSans DejaVuSans-Bold; do
    cp "$sys/usr/share/fonts/dejavu/$f.ttf" "$fonts/"
done
touch "$out/stage/.stamp"
echo "arcan/build.sh: done ($(du -sh "$out/stage" | cut -f1) staged)"
