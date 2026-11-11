#
# Machine dependent definitions for the aarch64 architecture.
#

CMAPS+=	${.CURDIR}/aarch64/Symbol.map

# Long double is IEEE quad precision
GDTOASRCS+=strtorQ.c
MDSRCS+=machdep_ldisQ.c
