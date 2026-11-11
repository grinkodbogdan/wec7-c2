/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 *
 * What a program needs from gcc's crtbegin.o once constructors and
 * destructors live in .init_array/.fini_array (as clang and lld emit
 * them): the DSO handle __cxa_atexit() uses to tell modules apart.
 * Built as crtbegin.o (programs: no handle) and with -DSHARED as
 * crtbeginS.o (shared objects: the address of their own handle).
 */
#ifdef SHARED
void *__dso_handle = &__dso_handle;
#else
void *__dso_handle = 0;
#endif
__asm(".hidden __dso_handle");
