# aarch64 gen sources; setjmp and friends from FreeBSD/arm64.

SRCS+=	_setjmp.S setjmp.S sigsetjmp.S \
	makecontext.c \
	fabs.S flt_rounds.c fpgetmask.c fpgetround.c fpsetmask.c \
	fpsetround.c infinity.c nan.c
