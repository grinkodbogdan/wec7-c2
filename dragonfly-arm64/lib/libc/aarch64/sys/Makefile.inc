# aarch64 system call glue.

MDASM=	vfork.S cerror.S getcontext.S pipe.S pipe2.S setlogin.S fake_sbrk.S

# Don't generate default code for these syscalls:
NOASM=	exit.o ftruncate.o getlogin.o \
	lseek.o mmap.o openbsd_poll.o pread.o \
	pwrite.o sstk.o truncate.o vfork.o yield.o \
	fork.o

PSEUDO=	_getlogin.o _exit.o

SRCS+=	cerror_set.c

# getcontext.S needs certain offsets
SRCS+= asmcontext.h
CLEANFILES+= asmcontext.h asmcontext.o

GENASSYM= NM=${NM:Unm} sh ${.CURDIR}/../../sys/kern/genassym.sh

asmcontext.o: asmcontext.c
	${CC} ${CFLAGS} -fcommon -c ${.IMPSRC}

asmcontext.h: asmcontext.o
	${GENASSYM} ${.ALLSRC} > ${.TARGET}
