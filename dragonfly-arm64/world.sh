#!/bin/sh
#
# Build DragonFly userland for arm64: headers, compiler support library,
# C library, startup files and programs, installed into a sysroot.
#
# Usage: ./world.sh [sysroot|builtins|libc|csu|hello|progs|all]
#        ./world.sh lib DIR [cflags...] | prog DIR [cflags...]
#
# Needs what build.sh needs, plus bmake and rpcgen.
#
# DragonFly's makefiles are written for its own make(1) and a native
# toolchain.  bmake reads them well enough to answer "which sources,
# which flags", so this script asks it (bmake -V) and compiles with
# clang itself, the way build.sh stands in for config(8).
#
set -e

HERE=$(cd "$(dirname "$0")" && pwd)
DFLY_SRC=${DFLY_SRC:-$HERE/work/dragonfly}
OBJ=${OBJ:-$HERE/work/obj}
WOBJ=${WOBJ:-$HERE/work/wobj}
SYSROOT=${SYSROOT:-$HERE/work/sysroot}
JOBS=${JOBS:-$(nproc 2>/dev/null || echo 2)}
LLVM_REPO=${LLVM_REPO:-https://github.com/llvm/llvm-project.git}
LLVM_TAG=${LLVM_TAG:-llvmorg-18.1.3}

S=$DFLY_SRC/sys
INC=$SYSROOT/usr/include

# The compiler driver.  clang has no aarch64 DragonFly target that
# defines __DragonFly__, so use a bare ELF target, say who we are, and
# use only the sysroot's headers.  wchar_t is int on DragonFly.
CC_FLAGS="--target=aarch64-unknown-none-elf -D__DragonFly__=6 -D__ELF__"
CC_FLAGS="$CC_FLAGS -nostdinc -isystem $INC -isystem $(clang -print-resource-dir)/include"
CC_FLAGS="$CC_FLAGS -Xclang -fwchar-type=int -Xclang -fsigned-wchar"
CC_FLAGS="$CC_FLAGS -fno-common -ffunction-sections -fdata-sections"
cc() { clang $CC_FLAGS "$@"; }

bm() {	# bmake in a DragonFly source directory
	dir=$1; shift
	(cd "$DFLY_SRC/$dir" && bmake -m "$DFLY_SRC/share/mk" \
	    MACHINE=arm64 MACHINE_ARCH=aarch64 MACHINE_PLATFORM=arm64 \
	    "$@" 2>/dev/null)
}

#
# Headers, laid out the way include/Makefile installs them.
#
sysroot() {
	"$HERE/build.sh" fetch
	# Refresh in place: libraries built later install headers here too.
	mkdir -p "$INC" "$SYSROOT/usr/lib"
	cd "$DFLY_SRC/include"
	incs=$(bmake -m "$DFLY_SRC/share/mk" -V INCS MACHINE_ARCH=aarch64 2>/dev/null)
	for h in $incs; do
		[ -f "$h" ] && cp "$h" "$INC/"
	done
	for d in arpa protocols rpc rpcsvc xlocale; do
		mkdir -p "$INC/$d"
		cp "$d"/*.h "$d"/*.x "$INC/$d/" 2>/dev/null || :
	done
	# rpcsvc and rpc protocol headers generated from .x files.
	for x in rpcsvc/*.x rpc/*.x; do
		rpcgen -C -h -o "$INC/${x%.x}.h" "$x" 2>/dev/null || :
	done
	# osreldate.h
	rel=$(awk '/^#define[[:space:]]+__DragonFly_version/ {print $3}' \
	    "$S/sys/param.h")
	printf '#ifdef _KERNEL\n#error "osreldate.h must not be used in the kernel, use sys/param.h"\n#else\n#undef __DragonFly_version\n#define __DragonFly_version %s\n#endif\n' \
	    "$rel" > "$INC/osreldate.h"

	ldirs=$(bmake -m "$DFLY_SRC/share/mk" -V LDIRS 2>/dev/null)
	lsub=$(bmake -m "$DFLY_SRC/share/mk" -V LSUBDIRS 2>/dev/null)
	for d in $ldirs $lsub; do
		mkdir -p "$INC/$d"
		cp "$S/$d"/*.h "$INC/$d/" 2>/dev/null || :
	done
	mkdir -p "$INC/dev/acpica" "$INC/dev/misc/evdev"
	cp "$S/dev/acpica/acpiio.h" "$INC/dev/acpica/"
	cp "$S"/dev/misc/evdev/input*.h "$INC/dev/misc/evdev/"
	# <sys/foo.h> also reachable as <foo.h>, as INCSLINKS arranges.
	for h in aio.h errno.h fcntl.h linker_set.h poll.h sched.h \
	    syslog.h ucontext.h; do
		ln -sf "sys/$h" "$INC/$h"
	done
	ln -sf ieeefp.h "$INC/floatingpoint.h"
	ln -sf netproto/smb "$INC/netsmb"
	# cpu/ from the cpu architecture; machine/ is cpu/ overlaid with
	# the platform's headers.
	# Headers other libraries install: libm's (OpenBSD's) and libutil's.
	cp "$DFLY_SRC"/contrib/openbsd_libm/include/global/*.h "$INC/"
	mkdir -p "$INC/machine"
	cp "$DFLY_SRC"/contrib/openbsd_libm/include/arm64/*.h "$INC/machine/"
	cp "$DFLY_SRC"/lib/libutil/libutil.h "$DFLY_SRC"/lib/libutil/login_cap.h "$INC/"
	ln -sf libutil.h "$INC/util.h"
	rpcgen -C -h -o "$INC/rpc/key_prot.h" rpcsvc/key_prot.x 2>/dev/null ||
	    (cd "$DFLY_SRC/include" && rpcgen -C -h -o "$INC/rpc/key_prot.h" rpcsvc/key_prot.x)
	mkdir -p "$INC/cpu" "$INC/machine"
	cp "$S"/cpu/aarch64/include/*.h "$INC/cpu/"
	cp "$S"/cpu/aarch64/include/*.h "$INC/machine/"
	cp "$S"/platform/arm64/include/*.h "$INC/machine/"
	# clang's own headers (arm_neon.h and friends) inside the sysroot,
	# where Meson's include-path filtering leaves them alone.
	rm -rf "$SYSROOT/usr/lib/clang/include"
	mkdir -p "$SYSROOT/usr/lib/clang"
	cp -r "$(clang -print-resource-dir)/include" "$SYSROOT/usr/lib/clang/include"
	echo "sysroot headers in $INC"
}

#
# compiler-rt builtins: what libgcc provides for gcc (128-bit integer
# and IEEE quad long double arithmetic, among others).
#
builtins() {
	src=$HERE/work/compiler-rt
	if [ ! -d "$src/lib/builtins" ]; then
		rm -rf "$src.git"
		git clone -q --depth 1 --branch "$LLVM_TAG" --filter=blob:none \
		    --sparse "$LLVM_REPO" "$src.git"
		git -C "$src.git" sparse-checkout set compiler-rt/lib/builtins
		mv "$src.git/compiler-rt" "$src"
		rm -rf "$src.git"
	fi
	o=$WOBJ/builtins
	rm -rf "$o"; mkdir -p "$o"
	b=$src/lib/builtins
	# Generic sources, minus what needs an OS service or another
	# architecture; plus the aarch64 directory.
	ls "$b"/*.c "$b"/aarch64/*.c "$b"/aarch64/*.S 2>/dev/null |
	    grep -vE '/(atomic|emutls|enable_execute_stack|gcc_personality_v0|clear_cache|eprintf|os_version_check|cpu_model|trampoline_setup|apple_versioning)\.c$' |
	    grep -vE '/aarch64/(lse|sme-|fp_mode)' |
	    grep -vE '/(x86|i386|x86_64|ppc|riscv|hexagon|arm|avr|loongarch|ve)/' |
	    grep -vE '/(.*xf.*|.*tc.*|.*bf.*)\.c$' > "$o/list"
	xargs -P "$JOBS" -I{} sh -c '
		f={}; n=$(echo "$f" | sed "s|.*/builtins/||; s|/|_|g; s|\.[cS]$||")
		'"clang $CC_FLAGS"' -O2 -fPIC -fvisibility=hidden \
		    -DCOMPILER_RT_HAS_FLOAT16 -c "$f" -o "'"$o"'/$n.o" 2>"'"$o"'/$n.err" ||
		    echo "builtins: $f failed" >&2' < "$o/list"
	llvm-ar rcs "$SYSROOT/usr/lib/libcompiler_rt.a" "$o"/*.o
	ln -sf libcompiler_rt.a "$SYSROOT/usr/lib/libgcc.a"
	echo "built $SYSROOT/usr/lib/libcompiler_rt.a ($(ls "$o"/*.o | wc -l) objects)"
}

#
# Compile a library directory's SRCS into $WOBJ/<dir>, using bmake to
# find sources (.PATH), flags (CFLAGS) and generated files.
#
#   buildlib DIR NAME [extra cflags]
#
buildlib() {
	dir=$1 name=$2; shift 2
	o=$WOBJ/$dir
	rm -rf "$o"; mkdir -p "$o"
	srcs=$(bm "$dir" -V SRCS)
	vpath=$(bm "$dir" -V .PATH)
	# gcc-only warning options and -Werror stay out: a first port
	# builds with whatever clang warns about, logged per file.
	cflags=$(bm "$dir" -V CFLAGS | sed "s|-I\${.OBJDIR}|-I$o|g" |
	    sed -E 's/ -Werror / /; s/ -Wsystem-headers / /')
	cflags="$cflags -Wno-unknown-warning-option"
	echo "$srcs" | tr ' \t' '\n\n' | grep -v '^$' > "$o/srcs"
	echo "$vpath" | tr ' ' '\n' | grep -v '^$' > "$o/vpath"
	echo "$cflags" > "$o/cflags"
	# Per-file flags (bsd.lib.mk's FLAGS_GROUPS): "file flags..." lines.
	: > "$o/fileflags"
	for g in $(bm "$dir" -V FLAGS_GROUPS); do
		gf=$(bm "$dir" -V "${g}_FLAGS" | sed "s|\${.CURDIR}|$DFLY_SRC/$dir|g")
		for f in $(bm "$dir" -V "${g}_FLAGS_FILES"); do
			echo "$f $gf" >> "$o/fileflags"
		done
	done
	echo "$dir: $(wc -l < "$o/srcs") sources"
}

# Re-overlay the port onto the source tree and refresh the
# machine-dependent headers in the sysroot.
sync() {
	"$HERE/build.sh" fetch >/dev/null
	cp "$S"/cpu/aarch64/include/*.h "$INC/cpu/"
	cp "$S"/cpu/aarch64/include/*.h "$INC/machine/"
	cp "$S"/platform/arm64/include/*.h "$INC/machine/"
	cp "$DFLY_SRC"/contrib/openbsd_libm/include/arm64/*.h "$INC/machine/"
}

libc() {
	sync
	o=$WOBJ/lib/libc
	buildlib lib/libc c
	d=$DFLY_SRC/lib/libc
	cflags="$(cat "$o/cflags") -I$o -I$d/include -I$d/aarch64 -I$d/locale -I$d/thread/aarch64"

	# Generated sources: syscall stubs, gdtoa links, asmcontext.h.
	gen_targets=$(bm lib/libc -V SASM -V SPSEUDO | tr ' \n' '  ')
	for t in $(bm lib/libc -V SASM); do
		printf '#include "SYS.h"\nRSYSCALL(%s)\n' "${t%.S}" > "$o/$t"
	done
	for t in $(bm lib/libc -V SPSEUDO); do
		n=${t%.S}; n=${n#_}
		printf '#include "SYS.h"\nPSEUDO(%s)\n' "$n" > "$o/$t"
	done
	for t in $(bm lib/libc -V GDTOASRCS); do
		ln -sf "$DFLY_SRC/contrib/gdtoa/$t" "$o/gdtoa_$t"
	done
	# rpcgen, lex and yacc output: let bmake run its own rules.
	(cd "$d" && MAKEOBJDIR=$o bmake -m "$DFLY_SRC/share/mk" \
	    MACHINE=arm64 MACHINE_ARCH=aarch64 MACHINE_PLATFORM=arm64 \
	    DESTDIR="$SYSROOT" crypt.h crypt_clnt.c crypt_xdr.c yp.h yp_xdr.c nslexer.c \
	    nsparser.c >"$o/generate.log" 2>&1) || {
		cat "$o/generate.log"; return 1; }
	cc $cflags -O2 -fcommon -c "$d/aarch64/sys/asmcontext.c" \
	    -o "$o/asmcontext.o"
	NM=llvm-nm sh "$S/kern/genassym.sh" "$o/asmcontext.o" > "$o/asmcontext.h"

	compile_list "$o" "$cflags -O2 -DNO_COMPAT7"
	llvm-ar rcs "$SYSROOT/usr/lib/libc.a" $(sed 's|$|.o|; s|^|'"$o"'/|' "$o/objs")
	echo "built $SYSROOT/usr/lib/libc.a"
}

# Compile every source in $1/srcs, looking for each in $1 and then
# along $1/vpath.  Object names go to $1/objs.
compile_list() {
	o=$1 flags=$2
	: > "$o/objs"; : > "$o/errors"
	while read -r f; do
		case "$f" in *.h) continue ;; esac
		p=
		if [ -e "$o/$f" ]; then p=$o/$f; else
			while read -r v; do
				[ -e "$v/$f" ] && { p=$v/$f; break; }
			done < "$o/vpath"
		fi
		if [ -z "$p" ]; then echo "missing source: $f" >> "$o/errors"; continue; fi
		extra=$(awk -v f="$f" '$1 == f { $1 = ""; print; exit }' "$o/fileflags" 2>/dev/null)
		echo "${f%.*} $p $extra" | sed 's/[[:space:]]*$//'
	done < "$o/srcs" > "$o/plan"
	cut -d' ' -f1 "$o/plan" > "$o/objs"
	cat > "$o/cc1.sh" <<-EOF
	#!/bin/sh
	n=\$1 p=\$2; shift 2
	clang $CC_FLAGS $flags "\$@" -c "\$p" -o "$o/\$n.o" 2> "$o/\$n.log" ||
	    { echo "\$n" >> "$o/failed"; }
	EOF
	chmod +x "$o/cc1.sh"
	rm -f "$o/failed"
	xargs -P "$JOBS" -L 1 "$o/cc1.sh" < "$o/plan"
	if [ -s "$o/errors" ] || [ -s "$o/failed" ]; then
		cat "$o/errors" 2>/dev/null
		[ -s "$o/failed" ] && echo "failed: $(wc -l < "$o/failed") (logs in $o/*.log)" &&
		    head -50 "$o/failed"
		return 1
	fi
}

#
# Startup files: crt1.o (and Scrt1.o for PIE), crti.o/crtn.o around
# .init/.fini, crtbegin/crtend (from gcc elsewhere; see crtbegin.c).
#
csu() {
	sync
	o=$WOBJ/lib/csu; rm -rf "$o"; mkdir -p "$o"
	d=$DFLY_SRC/lib/csu
	f="-O2 -I$d/common -I$DFLY_SRC/lib/libc/include -fno-omit-frame-pointer
	    -fno-asynchronous-unwind-tables -Wall"
	# crt1.c goes through assembly so the ABI tag note can be marked
	# SHT_NOTE (see lib/csu/common/crtbrand.c).
	for v in "crt1" "Scrt1 -fPIC -DPIC" "gcrt1 -DGCRT"; do
		set -- $v; n=$1; shift
		cc $f "$@" -S -o "$o/$n.s" "$d/aarch64/crt1.c"
		sed -i -e '/\.note\.tag/s/progbits/note/' \
		    -e '/\.note\.tag.*"a"$/s/$/,@note/' "$o/$n.s"
		cc -c -o "$o/$n.o" "$o/$n.s"
	done
	cc $f -c -o "$o/crti.o" "$d/aarch64/crti.S"
	cc $f -c -o "$o/crtn.o" "$d/aarch64/crtn.S"
	cc $f -c -o "$o/crtbegin.o" "$d/aarch64/crtbegin.c"
	cc $f -fPIC -DSHARED -c -o "$o/crtbeginS.o" "$d/aarch64/crtbegin.c"
	cc $f -c -o "$o/crtend.o" "$d/aarch64/crtend.c"
	cp "$o/crtend.o" "$o/crtendS.o"
	cp "$o"/*.o "$SYSROOT/usr/lib/"
	# pthreads and dlopen live in libc on DragonFly; programs and
	# libraries that ask for -lpthread, -ldl or -lrt get empty archives.
	for l in pthread dl rt atomic; do
		llvm-ar rcs "$SYSROOT/usr/lib/lib$l.a"
	done
	echo "built startup files in $SYSROOT/usr/lib"
}

# Link a static program: ldstatic OUTPUT OBJECT... [-lfoo...]
ldstatic() {
	out=$1; shift
	l=$SYSROOT/usr/lib
	ld.lld -static -nostdlib --eh-frame-hdr -o "$out" "$l/crt1.o" "$l/crti.o" \
	    "$l/crtbegin.o" "$@" -L"$l" --start-group -lc -lcompiler_rt \
	    --end-group "$l/crtend.o" "$l/crtn.o"
}

#
# Any other library: lib DIR [extra cflags...]
# Installs lib<LIB>.a and the library's INCS into the sysroot.
#
lib() {
	sync
	dir=$1; shift
	name=$(bm "$dir" -V LIB)
	buildlib "$dir" "$name"
	o=$WOBJ/$dir
	cflags="$(cat "$o/cflags") -I$o $*"
	compile_list "$o" "$cflags"
	rm -f "$SYSROOT/usr/lib/lib$name.a"
	llvm-ar rcs "$SYSROOT/usr/lib/lib$name.a" $(sed 's|$|.o|; s|^|'"$o"'/|' "$o/objs")
	for h in $(bm "$dir" -V INCS); do
		[ -f "$DFLY_SRC/$dir/$h" ] && cp "$DFLY_SRC/$dir/$h" "$INC/"
	done
	echo "built $SYSROOT/usr/lib/lib$name.a"
}

#
# A program: prog DIR [extra cflags...]
# Links statically against the libraries its LDADD names; private
# libraries (/usr/lib/priv) are left out until they are built.
#
prog() {
	sync
	dir=$1; shift
	name=$(bm "$dir" -V PROG)
	buildlib "$dir" "$name"
	o=$WOBJ/$dir
	cflags="$(cat "$o/cflags") -I$o $*"
	compile_list "$o" "$cflags"
	libs=$(bm "$dir" -V LDADD | tr ' ' '\n' | grep '^-l' | grep -v private | tr '\n' ' ')
	ldstatic "$o/$name" $(sed 's|$|.o|; s|^|'"$o"'/|' "$o/objs") $libs
	mkdir -p "$SYSROOT/$(dirname "$dir" | sed 's|usr.bin|usr/bin|; s|usr.sbin|usr/sbin|')"
	cp "$o/$name" "$SYSROOT/$(dirname "$dir" | sed 's|usr.bin|usr/bin|; s|usr.sbin|usr/sbin|')/"
	echo "built $dir"
}

#
# The C++ runtime: LLVM's libunwind, libc++abi and libc++, static, with
# patches/llvm/libcxx-dragonfly.diff (DragonFly takes FreeBSD's paths in
# libc++'s locale and threading code).
#
cxx() {
	sync
	src=$HERE/work/llvm-runtimes
	if [ ! -d "$src/libcxx" ]; then
		git clone -q --depth 1 --branch "$LLVM_TAG" --filter=blob:none \
		    --sparse "$LLVM_REPO" "$src"
		git -C "$src" sparse-checkout set runtimes libcxx libcxxabi \
		    libunwind cmake llvm/cmake llvm/utils/llvm-lit third-party
		git -C "$src" apply "$HERE/patches/llvm/libcxx-dragonfly.diff"
	fi
	b=$WOBJ/llvm-runtimes
	cmake -G Ninja -S "$src/runtimes" -B "$b" \
	    -DCMAKE_TOOLCHAIN_FILE="$HERE/tools/aarch64-dragonfly.cmake" \
	    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$SYSROOT/usr" \
	    -DLLVM_ENABLE_RUNTIMES="libunwind;libcxxabi;libcxx" \
	    -DLIBUNWIND_ENABLE_SHARED=OFF -DLIBCXXABI_ENABLE_SHARED=OFF \
	    -DLIBCXX_ENABLE_SHARED=OFF -DLIBCXXABI_USE_LLVM_UNWINDER=ON \
	    -DLIBCXXABI_ENABLE_STATIC_UNWINDER=ON -DLIBCXX_CXX_ABI=libcxxabi \
	    -DLIBCXX_INCLUDE_BENCHMARKS=OFF -DLIBCXX_INCLUDE_TESTS=OFF \
	    -DLIBCXXABI_INCLUDE_TESTS=OFF -DLIBUNWIND_INCLUDE_TESTS=OFF \
	    -DLIBCXX_USE_COMPILER_RT=ON -DLIBCXXABI_USE_COMPILER_RT=ON \
	    -DLIBUNWIND_USE_COMPILER_RT=ON -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=ON \
	    -DLIBCXX_HARDENING_MODE=none >/dev/null
	ninja -C "$b" install >/dev/null
	echo "installed libc++, libc++abi, libunwind in $SYSROOT/usr"
}

# Meson cross file for the sysroot.
meson_cross() {
	sed "s|@SYSROOT@|$SYSROOT|g" "$HERE/tools/aarch64-dragonfly.meson.in" \
	    > "$WOBJ/aarch64-dragonfly.meson"
}

#
# Mesa: OpenGL with the softpipe software renderer, as a static OSMesa
# (no window system, no kernel graphics driver, no dynamic loading).
# Meson insists on a shared libOSMesa, so the archives that link would
# use are merged into libOSMesa.a instead.
#
MESA_VERSION=${MESA_VERSION:-24.0.9}
mesa() {
	sync
	src=$HERE/work/mesa-$MESA_VERSION
	if [ ! -d "$src" ]; then
		curl -sSfL "https://archive.mesa3d.org/mesa-$MESA_VERSION.tar.xz" |
		    tar xJf - -C "$HERE/work"
	fi
	meson_cross
	b=$WOBJ/mesa
	[ -f "$b/build.ninja" ] || meson setup "$b" "$src" \
	    --cross-file "$WOBJ/aarch64-dragonfly.meson" --prefix="$SYSROOT/usr" \
	    -Dbuildtype=release -Ddefault_library=static -Dplatforms= \
	    -Dgallium-drivers=swrast -Dvulkan-drivers= -Dosmesa=true \
	    -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dgles1=disabled \
	    -Dgles2=disabled -Dllvm=disabled -Dshader-cache=disabled \
	    -Dzlib=disabled -Dexpat=disabled -Dxmlconfig=disabled \
	    -Dzstd=disabled -Dvalgrind=disabled -Dlibunwind=disabled \
	    -Dlmsensors=disabled -Dbuild-tests=false -Dshared-glapi=disabled \
	    -Dglvnd=false -Dgallium-vdpau=disabled -Dgallium-va=disabled \
	    -Dgallium-xa=disabled -Dgallium-nine=false \
	    -Dgallium-opencl=disabled -Dandroid-libbacktrace=disabled \
	    -Dmicrosoft-clc=disabled -Dintel-clc=disabled -Dtools= \
	    -Dperfetto=false -Ddraw-use-llvm=false >/dev/null
	ninja -C "$b" > "$b/build.log" 2>&1 || :	# the .so link fails
	(cd "$b" && ninja -t commands \
	    src/gallium/targets/osmesa/libOSMesa.so.8.0.0 | tail -1 |
	    tr ' ' '\n' | grep -E '\.(a|o)$' | grep -v '/crt' | sort -u \
	    > osmesa-inputs.txt &&
	    rm -f libOSMesa.a && llvm-ar qcL libOSMesa.a $(cat osmesa-inputs.txt) &&
	    llvm-ranlib libOSMesa.a)
	cp "$b/libOSMesa.a" "$SYSROOT/usr/lib/"
	mkdir -p "$INC/GL" "$INC/KHR"
	cp "$src"/include/GL/osmesa.h "$src"/include/GL/gl.h \
	    "$src"/include/GL/glext.h "$INC/GL/"
	cp "$src/include/KHR/khrplatform.h" "$INC/KHR/"
	echo "built $SYSROOT/usr/lib/libOSMesa.a"
}

# FreeType, for Arcan's text rendering.
FREETYPE_VERSION=${FREETYPE_VERSION:-2.13.2}
freetype() {
	src=$HERE/work/freetype-$FREETYPE_VERSION
	[ -d "$src" ] || curl -sSfL \
	    "https://download.savannah.gnu.org/releases/freetype/freetype-$FREETYPE_VERSION.tar.xz" |
	    tar xJf - -C "$HERE/work"
	b=$WOBJ/freetype
	cmake -G Ninja -S "$src" -B "$b" \
	    -DCMAKE_TOOLCHAIN_FILE="$HERE/tools/aarch64-dragonfly.cmake" \
	    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$SYSROOT/usr" \
	    -DFT_DISABLE_ZLIB=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON \
	    -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_BROTLI=ON >/dev/null
	ninja -C "$b" install >/dev/null
	echo "installed freetype in $SYSROOT/usr"
}

#
# Arcan 0.7.0.1 (the last C release, OpenGL based) with the fbmem video
# platform from ./arcan (OSMesa rendering, /dev/fb0 output), evdev input,
# no audio, bundled Lua 5.1 and SQLite, all static.
# patches/arcan/arcan-dragonfly.diff: the fbmem platform in the build,
# GL symbols resolved through the platform (no dlsym in static
# binaries), static shmif/tui/a12 libraries, DragonFly in evdev's BSD
# paths.
#
ARCAN_TAG=${ARCAN_TAG:-0.7.0.1}
arcan() {
	src=$HERE/work/arcan-$ARCAN_TAG
	if [ ! -d "$src" ]; then
		git clone -q --depth 1 --branch "$ARCAN_TAG" \
		    https://github.com/letoram/arcan.git "$src"
		git -C "$src" apply "$HERE/patches/arcan/arcan-dragonfly.diff"
	fi
	cp -R "$HERE/arcan/src/." "$src/src/"
	mkdir -p "$INC/linux"
	printf '#include <dev/misc/evdev/input.h>\n' > "$INC/linux/input.h"
	b=$WOBJ/arcan
	cmake -G Ninja -S "$src/src" -B "$b" \
	    -DCMAKE_TOOLCHAIN_FILE="$HERE/tools/aarch64-dragonfly.cmake" \
	    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr \
	    -DVIDEO_PLATFORM=fbmem -DAGP_PLATFORM=gl21 -DAUDIO_PLATFORM=stub \
	    -DBUILTIN_LUA=ON -DDISABLE_JIT=ON -DLUA_FOUND=TRUE \
	    -DLUA_LIBRARY=lua51 -DLUA_INCLUDE_DIR="$src/external/lua" \
	    -DSTATIC_SQLite3=ON -DENABLE_LWA=OFF -DDISABLE_WAYLAND=ON \
	    -DSTATIC_LIBUVC=OFF -DDISABLE_FSRV_GAME=ON -DDISABLE_FSRV_DECODE=ON \
	    -DDISABLE_FSRV_NET=ON -DDISABLE_FSRV_ENCODE=ON \
	    -DDISABLE_FSRV_REMOTING=ON -DDISABLE_FSRV_AVFEED=ON \
	    -DENABLE_SIMD=OFF -DARCAN_SHMIF_LIBTYPE=STATIC \
	    -DOPENGL_gl_LIBRARY="$SYSROOT/usr/lib/libOSMesa.a" \
	    -DOPENGL_opengl_LIBRARY="$SYSROOT/usr/lib/libOSMesa.a" \
	    -DOPENGL_INCLUDE_DIR="$INC" -DOPENGL_GLX_INCLUDE_DIR="$INC" \
	    -DDFLY_EXTRA_LIBS="-lOSMesa -lc++ -lc++abi -lunwind -lm" >/dev/null
	ninja -C "$b" arcan arcan_frameserver afsrv_terminal >/dev/null
	echo "built arcan, arcan_frameserver, afsrv_terminal in $b"
}

# A smoke test: a small program using stdio, malloc, TLS and threads.
hello() {
	o=$WOBJ/hello; mkdir -p "$o"
	for p in hello keymenu; do
		cc -O2 -Wall -c "$HERE/user/$p.c" -o "$o/$p.o"
		ldstatic "$o/$p" "$o/$p.o"
		echo "built $o/$p"
	done
}

# The libraries and programs the ramdisk carries.  ls is built without
# colour and sh without line editing until ncurses and libedit are.
LIBS="lib/libdmsg lib/libutil lib/libcrypt lib/libm lib/libkvm lib/libkinfo"
PROGS="bin/cat bin/chmod bin/cp bin/date bin/dd bin/df bin/echo bin/hostname
	bin/kill bin/ln bin/mkdir bin/mv bin/ps bin/pwd bin/rm bin/sleep
	bin/stty bin/test sbin/dmesg sbin/init sbin/mount sbin/umount sbin/mount_tmpfs sbin/reboot
	sbin/sysctl sbin/mount_ext2fs usr.sbin/vnconfig usr.bin/env usr.bin/head usr.bin/tail usr.bin/uname
	usr.bin/wc usr.bin/tr usr.bin/cmp bin/sync sbin/newfs_hammer2 sbin/mount_hammer2"
progs() {
	for l in $LIBS; do lib "$l"; done
	for p in $PROGS; do prog "$p"; done
	prog bin/ls -UCOLORLS
	prog bin/sh -DNO_HISTORY
}

case "${1:-all}" in
progs)		progs ;;
csu)		csu ;;
cxx)		cxx ;;
mesa)		mesa ;;
freetype)	freetype ;;
arcan)		arcan ;;
lib)		shift; lib "$@" ;;
prog)		shift; prog "$@" ;;
hello)		hello ;;
sysroot)	sysroot ;;
builtins)	builtins ;;
libc)		libc ;;
all)		sysroot; builtins; libc; csu; hello; progs ;;
*)	echo "usage: $0 [sysroot|builtins|libc|all]" >&2; exit 1 ;;
esac
