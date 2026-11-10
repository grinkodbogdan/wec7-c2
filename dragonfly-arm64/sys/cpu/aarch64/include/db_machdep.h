/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 * 1. Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in
 *    the documentation and/or other materials provided with the
 *    distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
 * ANY EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.  IN NO EVENT SHALL THE
 * AUTHOR OR CONTRIBUTORS BE LIABLE FOR ANY DAMAGES ARISING IN ANY WAY OUT
 * OF THE USE OF THIS SOFTWARE.
 */

#ifndef	_CPU_DB_MACHDEP_H_
#define	_CPU_DB_MACHDEP_H_

#include <cpu/armreg.h>
#include <cpu/frame.h>
#include <cpu/trap.h>

typedef	vm_offset_t	db_addr_t;	/* address - unsigned */
typedef	long		db_expr_t;	/* expression - signed */

typedef struct trapframe db_regs_t;

#ifdef _KERNEL
#define	DDB_EXPR_FMT	"l"		/* expression is long */
extern db_regs_t        ddb_regs;       /* register state */
#define DDB_REGS        (&ddb_regs)

db_addr_t		PC_REGS(db_regs_t *);
db_addr_t		SP_REGS(db_regs_t *);
db_addr_t		BP_REGS(db_regs_t *);
#endif

#define	BKPT_INST	0xd4200000	/* brk #0 */
#define	BKPT_SIZE	(4)		/* size of breakpoint inst */
#define	BKPT_SET(inst)	(BKPT_INST)

/* ELR already points at the BRK instruction itself. */
#define FIXUP_PC_AFTER_BREAK

/* MDSCR_EL1.SS drives single stepping together with SPSR.SS (bit 21). */
#define db_clear_single_step(regs)	((regs)->tf_spsr &= ~(1UL << 21))
#define db_set_single_step(regs)	((regs)->tf_spsr |=  (1UL << 21))

#define	IS_BREAKPOINT_TRAP(type, code)	((type) == T_BPTFLT)
#define IS_WATCHPOINT_TRAP(type, code)	0

#define	inst_trap_return(ins)	((ins) == 0xd69f03e0)		/* eret */
#define	inst_return(ins)	(((ins) & 0xfffffc1f) == 0xd65f0000) /* ret */
#define	inst_call(ins)		(((ins) & 0xfc000000) == 0x94000000 || \
				 ((ins) & 0xfffffc1f) == 0xd63f0000) /* bl, blr */
#define inst_load(ins)		0
#define inst_store(ins)		0

#define	DB_SMALL_VALUE_MAX	0x7fffffff
#define	DB_SMALL_VALUE_MIN	(-0x400001)

#endif /* !_CPU_DB_MACHDEP_H_ */
