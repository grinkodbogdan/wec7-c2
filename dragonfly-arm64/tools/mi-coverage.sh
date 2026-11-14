#!/bin/sh
#
# Compile every "standard" machine-independent kernel source file (the
# files sys/conf/files says go into every kernel) for aarch64 and report
# which ones build.  This is the yardstick for the cpu/aarch64 and
# platform/arm64 headers: the port cannot link a real kernel until all
# of these compile.
#
# Usage: tools/mi-coverage.sh [-v] [file ...]
#   -v		print the first error of every failing file
#   file	limit the run to these sources (paths relative to sys/)
#
# Environment: DFLY_SRC, OBJ, CC as for build.sh (run "build.sh build"
# first so the tree is fetched and the overlay applied).
#
HERE=$(cd "$(dirname "$0")/.." && pwd)
DFLY_SRC=${DFLY_SRC:-$HERE/work/dragonfly}
OBJ=${OBJ:-$HERE/work/obj}
CC=${CC:-clang}
JOBS=${JOBS:-$(nproc 2>/dev/null || echo 2)}
S=$DFLY_SRC/sys
P=$S/platform/arm64
C=$OBJ/coverage
verbose=0
[ "$1" = "-v" ] && { verbose=1; shift; }

cp -R "$HERE/sys/." "$S/"
rm -rf "$C"
mkdir -p "$C/include/machine" "$C/opt" "$C/out"
ln -s "$S/cpu/aarch64/include" "$C/include/cpu"
for h in "$S"/cpu/aarch64/include/*.h "$P"/include/*.h; do
	ln -sf "$h" "$C/include/machine/$(basename "$h")"
done

# config(8) would generate these from the kernel configuration file.
# Empty option headers select every option's default; a device count of
# zero means "driver not configured".
grep -rhoE '#include[[:space:]]+"opt_[a-zA-Z0-9_]+\.h"' "$S" 2>/dev/null |
    sed -E 's/.*"(.*)"/\1/' | sort -u | while read -r h; do : > "$C/opt/$h"; done
: > "$C/opt/opt_global.h"
echo "#define MAXUSERS 0" > "$C/opt/opt_maxusers.h"
grep -rhoE '#include[[:space:]]+"use_[a-zA-Z0-9_]+\.h"' "$S" 2>/dev/null |
    sed -E 's/.*"(.*)"/\1/' | sort -u | while read -r h; do
	n=$(echo "${h#use_}" | sed 's/\.h$//' | tr a-z A-Z)
	echo "#define N$n 0" > "$C/opt/$h"
done
# What a GENERIC-style kernel configuration turns on.
cat > "$C/opt/opt_global.h" <<EOG
#define INVARIANTS 1
EOG
echo "#define INET 1" > "$C/opt/opt_inet.h"
echo "#define INET6 1" > "$C/opt/opt_inet6.h"

# Interface headers generated from the .m files at build time.
for m in "$S"/kern/*.m "$S"/libiconv/*.m; do
	[ -f "$m" ] && (cd "$C/opt" && awk -f "$S/tools/makeobjops.awk" -- -h "$m")
done

CFLAGS="--target=aarch64-unknown-none-elf -D__DragonFly__ -D_KERNEL \
	-DHAVE_KERNEL_OPTION_HEADERS -include opt_global.h \
	-O2 -std=gnu99 -ffreestanding -fno-pic -mgeneral-regs-only \
	-fno-strict-aliasing -fno-stack-protector -w -ferror-limit=5 \
	-nostdinc -I$C/opt -I$C/include -I$S"

if [ $# -gt 0 ]; then
	for f in "$@"; do echo "$f"; done > "$C/files"
else
	awk '$2 == "standard" && $1 ~ /^[a-z].*\.c$/ { print $1 }' "$S/conf/files" \
	    > "$C/files"
fi

export CC CFLAGS S C
xargs -P "$JOBS" -I{} sh -c '
	o=$(echo "{}" | tr / _)
	if $CC $CFLAGS -c "$S/{}" -o "$C/out/$o.o" 2> "$C/out/$o.err"; then
		echo "{}" >> "$C/pass"
	else
		echo "{}" >> "$C/fail"
	fi' < "$C/files"

touch "$C/pass" "$C/fail"
total=$(wc -l < "$C/files")
pass=$(wc -l < "$C/pass")
echo "MI coverage: $pass / $total standard kernel files compile for aarch64"
echo
echo "Most common first errors:"
for f in $(cat "$C/fail"); do
	grep -m1 -E "error:" "$C/out/$(echo "$f" | tr / _).err"
done | sed -E 's/^[^ ]*: (fatal )?error: //' | sort | uniq -c | sort -rn | head -${TOP:-25}

# With everything compiled, a relocatable link shows what the standard
# files still need from elsewhere: machine-dependent code that has not
# been written yet, plus optional and config-generated files.
if [ "$pass" = "$total" ] && [ $# -eq 0 ]; then
	LD=${LD:-ld.lld}
	NM=${NM:-llvm-nm}
	command -v "$NM" >/dev/null 2>&1 || NM=nm
	$LD -r -o "$C/mi.o" "$C"/out/*.o
	$NM -u "$C/mi.o" | awk '{ print $2 }' | sort -u > "$C/undefined"
	if [ -f "$OBJ/kernel.elf" ]; then
		$NM --defined-only "$OBJ/kernel.elf" | awk '{ print $3 }' |
		    sort -u > "$C/provided"
		comm -23 "$C/undefined" "$C/provided" > "$C/undefined.tmp"
		mv "$C/undefined.tmp" "$C/undefined"
	fi
	echo "(none)"
	echo
	echo "Unresolved symbols after linking them: $(wc -l < "$C/undefined")"
	echo "  list written to $C/undefined"
fi
if [ $verbose = 1 ]; then
	echo
	for f in $(sort "$C/fail"); do
		printf '%s: ' "$f"
		grep -m1 -E "error:" "$C/out/$(echo "$f" | tr / _).err"
	done
fi
