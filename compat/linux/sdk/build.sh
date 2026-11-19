#!/bin/sh
# compat/linux/sdk/build.sh OUT - a C toolchain for Linux arm64 test programs:
# musl 1.2.6 built with clang, Linux UAPI headers, and the flare-a64-cc driver
# (static PIE programs). Adapted from QRT's sdk/build-arm64.sh
# (github.com/grinkodbogdan/qrt).
#
# Needs clang, ld.lld, llvm-ar and gcc-aarch64-linux-gnu (for libgcc) plus
# linux-libc-dev-arm64-cross (for <asm/...>).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$1
mkdir -p "$out/src" "$out/bin"
out=$(cd "$out" && pwd)
sys="$out/sysroot"
CC="clang --target=aarch64-linux-musl -fuse-ld=lld"
rm -rf "$out/src/musl" "$sys"
tar -xzf "$here/musl-1.2.6.tar.gz" -C "$out/src"
( cd "$out/src/musl" &&
  CC="$CC" AR=llvm-ar RANLIB=llvm-ranlib ./configure --target=aarch64 --prefix=/usr --syslibdir=/lib \
      CFLAGS="-O2 -g0" >/dev/null &&
  make -j"$(nproc)" >/dev/null &&
  make DESTDIR="$sys" install >/dev/null )
for d in linux asm-generic; do cp -r "/usr/include/$d" "$sys/usr/include/"; done
cp -r /usr/aarch64-linux-gnu/include/asm "$sys/usr/include/asm"
gcclib=$(dirname "$(aarch64-linux-gnu-gcc -print-libgcc-file-name)")
cp "$gcclib/libgcc.a" "$sys/usr/lib/libgcc.a"
cp "$gcclib/crtbeginS.o" "$gcclib/crtendS.o" "$gcclib/crtbegin.o" "$gcclib/crtend.o" "$gcclib/crtbeginT.o" "$sys/usr/lib/"
mkdir -p "$out/nogcc"
llvm-ar rcs "$sys/usr/lib/libgcc_eh.a"
echo "INPUT(-lgcc)" > "$sys/usr/lib/libgcc_s.so"   # libgcc is linked statically into every program
cat > "$out/bin/flare-a64-cc" <<EOS
#!/bin/sh
# flare-a64-cc: clang for Linux arm64 programs on Flare (static PIE, musl)
exec clang --target=aarch64-linux-musl --sysroot="$sys" --gcc-toolchain="$out/nogcc" -nostdlibinc \\
    -isystem "$sys/usr/include" -static-pie -fPIE -fuse-ld=lld --rtlib=libgcc -L"$sys/usr/lib" "\$@"
EOS
chmod +x "$out/bin/flare-a64-cc"
# Dynamically linked programs and shared libraries (musl's ld-musl-aarch64.so.1).
cat > "$out/bin/flare-a64-cc-dyn" <<EOS
#!/bin/sh
# flare-a64-cc-dyn: clang for dynamically linked Linux arm64 programs on Flare
exec clang --target=aarch64-linux-musl --sysroot="$sys" --gcc-toolchain="$out/nogcc" -nostdlibinc \\
    -isystem "$sys/usr/include" -fPIC -fuse-ld=lld --rtlib=libgcc -L"$sys/usr/lib" \\
    -Wl,-dynamic-linker,/lib/ld-musl-aarch64.so.1 "\$@"
EOS
chmod +x "$out/bin/flare-a64-cc-dyn"
touch "$out/.stamp"
