# CMake toolchain file for cross-building to DragonFly/arm64 against the
execute_process(COMMAND clang -print-resource-dir OUTPUT_VARIABLE CLANG_RESOURCE_DIR OUTPUT_STRIP_TRAILING_WHITESPACE)
# sysroot world.sh builds (work/sysroot).  clang has no aarch64 DragonFly
# target that defines __DragonFly__, so a bare ELF target is used with
# the definitions and the sysroot's headers added by hand, as in world.sh.
set(CMAKE_SYSTEM_NAME DragonFly)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(DFLY_SYSROOT "${CMAKE_CURRENT_LIST_DIR}/../work/sysroot" CACHE PATH "")
set(CMAKE_SYSROOT "${DFLY_SYSROOT}")
set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)
set(CMAKE_ASM_COMPILER clang)
set(CMAKE_AR llvm-ar)
set(CMAKE_RANLIB llvm-ranlib)
set(CMAKE_C_COMPILER_TARGET aarch64-unknown-none-elf)
set(CMAKE_CXX_COMPILER_TARGET aarch64-unknown-none-elf)
set(CMAKE_ASM_COMPILER_TARGET aarch64-unknown-none-elf)
set(_dfly "-D__DragonFly__=6 -D__ELF__ -Xclang -fwchar-type=int -Xclang -fsigned-wchar")
set(_inc "-isystem ${DFLY_SYSROOT}/usr/include -isystem ${CLANG_RESOURCE_DIR}/include")
set(CMAKE_C_FLAGS_INIT "${_dfly} -nostdinc ${_inc}")
set(CMAKE_ASM_FLAGS_INIT "${_dfly} -nostdinc ${_inc}")
# C++: libc++'s headers first (from the sysroot once installed), then C.
set(CMAKE_CXX_FLAGS_INIT "${_dfly} -nostdinc -isystem ${DFLY_SYSROOT}/usr/include/c++/v1 ${_inc}")
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=lld -nostdlib -static -Wl,--eh-frame-hdr ${DFLY_SYSROOT}/usr/lib/crt1.o ${DFLY_SYSROOT}/usr/lib/crti.o ${DFLY_SYSROOT}/usr/lib/crtbegin.o -L${DFLY_SYSROOT}/usr/lib")
set(CMAKE_C_STANDARD_LIBRARIES "-Wl,--start-group ${DFLY_EXTRA_LIBS} -lc -lcompiler_rt -Wl,--end-group ${DFLY_SYSROOT}/usr/lib/crtend.o ${DFLY_SYSROOT}/usr/lib/crtn.o")
set(CMAKE_CXX_STANDARD_LIBRARIES "-Wl,--start-group -lc++ -lc++abi -lunwind -lm -lc -lcompiler_rt -Wl,--end-group ${DFLY_SYSROOT}/usr/lib/crtend.o ${DFLY_SYSROOT}/usr/lib/crtn.o")
# Extra libraries for C links, e.g. -DDFLY_EXTRA_LIBS="-lOSMesa -lc++ -lc++abi
# -lunwind -lm" for C programs that pull in C++ code.
set(DFLY_EXTRA_LIBS "${DFLY_EXTRA_LIBS}" CACHE STRING "")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
set(BUILD_SHARED_LIBS OFF)
