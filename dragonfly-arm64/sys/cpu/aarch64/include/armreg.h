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

#ifndef _CPU_ARMREG_H_
#define	_CPU_ARMREG_H_

/*
 * aarch64 system register access and field definitions.  This is the
 * moral equivalent of x86_64's specialreg.h.
 */

#ifndef LOCORE
#define	READ_SPECIALREG(reg)						\
({	__uint64_t _val;						\
	__asm __volatile("mrs	%0, " __STRING(reg) : "=&r" (_val));	\
	_val;								\
})

#define	WRITE_SPECIALREG(reg, val)					\
	__asm __volatile("msr	" __STRING(reg) ", %0" : : "r" ((__uint64_t)(val)))
#endif

/* CurrentEL */
#define	CURRENTEL_EL(x)		(((x) >> 2) & 3)

/* DAIF: interrupt mask bits as they appear in SPSR / the DAIF register */
#define	PSR_F			0x00000040
#define	PSR_I			0x00000080
#define	PSR_A			0x00000100
#define	PSR_D			0x00000200
#define	PSR_DAIF		(PSR_D | PSR_A | PSR_I | PSR_F)
/* ... and as operands to "msr daifset/daifclr, #imm" */
#define	DAIF_F			0x1
#define	DAIF_I			0x2
#define	DAIF_A			0x4
#define	DAIF_D			0x8
#define	DAIF_INTR		(DAIF_I | DAIF_F)

/* SPSR_EL1 mode field */
#define	PSR_M_EL0t		0x00000000
#define	PSR_M_EL1t		0x00000004
#define	PSR_M_EL1h		0x00000005
#define	PSR_M_EL2h		0x00000009
#define	PSR_M_MASK		0x0000000f
#define	PSR_AARCH32		0x00000010
#define	PSR_FLAGS		0xf0000000	/* NZCV */
#define	PSR_C			0x20000000

/* ESR_ELx */
#define	ESR_ELx_ISS_MASK	0x01ffffff
#define	ESR_ELx_IL		(1 << 25)
#define	ESR_ELx_EC_SHIFT	26
#define	ESR_ELx_EC(esr)		(((esr) >> ESR_ELx_EC_SHIFT) & 0x3f)
#define	EXCP_UNKNOWN		0x00
#define	EXCP_TRAP_WFI_WFE	0x01
#define	EXCP_FP_SIMD		0x07	/* FP/SIMD access trapped */
#define	EXCP_ILL_STATE		0x0e
#define	EXCP_SVC32		0x11
#define	EXCP_SVC64		0x15	/* system call */
#define	EXCP_MSR		0x18	/* trapped MSR/MRS/SYS */
#define	EXCP_INSN_ABORT_L	0x20	/* instruction abort from lower EL */
#define	EXCP_INSN_ABORT		0x21	/* instruction abort from same EL */
#define	EXCP_PC_ALIGN		0x22
#define	EXCP_DATA_ABORT_L	0x24	/* data abort from lower EL */
#define	EXCP_DATA_ABORT		0x25	/* data abort from same EL */
#define	EXCP_SP_ALIGN		0x26
#define	EXCP_TRAP_FP		0x2c
#define	EXCP_SERROR		0x2f
#define	EXCP_BRKPT_EL0		0x30
#define	EXCP_SOFTSTP_EL0	0x32
#define	EXCP_SOFTSTP_EL1	0x33
#define	EXCP_WATCHPT_EL1	0x35
#define	EXCP_BRK		0x3c	/* BRK instruction */

/* ISS for data/instruction aborts */
#define	ISS_DATA_WnR		(1 << 6)
#define	ISS_DATA_DFSC_MASK	0x3f
#define	ISS_DATA_DFSC_TF_L0	0x04	/* translation fault, level 0-3 */
#define	ISS_DATA_DFSC_AFF_L1	0x09	/* access flag fault, level 1-3 */
#define	ISS_DATA_DFSC_PF_L1	0x0d	/* permission fault, level 1-3 */

/* SCTLR_EL1 */
#define	SCTLR_M			0x00000001	/* MMU enable */
#define	SCTLR_A			0x00000002	/* alignment check */
#define	SCTLR_C			0x00000004	/* D-cache enable */
#define	SCTLR_SA		0x00000008	/* SP alignment check */
#define	SCTLR_SA0		0x00000010
#define	SCTLR_I			0x00001000	/* I-cache enable */
#define	SCTLR_DZE		0x00004000	/* EL0 may use DC ZVA */
#define	SCTLR_UCT		0x00008000	/* EL0 may read CTR_EL0 */
#define	SCTLR_WXN		0x00080000
#define	SCTLR_UCI		0x04000000	/* EL0 cache maintenance */
#define	SCTLR_RES1		0x30d00800

/* TCR_EL1 */
#define	TCR_T0SZ_SHIFT		0
#define	TCR_EPD0		(1UL << 7)	/* disable TTBR0 walks */
#define	TCR_T1SZ_SHIFT		16
#define	TCR_A1			(1UL << 22)
#define	TCR_EPD1		(1UL << 23)
#define	TCR_IPS_SHIFT		32
#define	TCR_ASID_16		(1UL << 36)

/* CPACR_EL1 */
#define	CPACR_FPEN_MASK		(0x3UL << 20)
#define	CPACR_FPEN_TRAP_ALL	(0x0UL << 20)
#define	CPACR_FPEN_TRAP_EL0	(0x1UL << 20)
#define	CPACR_FPEN_TRAP_NONE	(0x3UL << 20)

/* MIDR_EL1 */
#define	MIDR_REVISION(x)	((x) & 0xf)
#define	MIDR_PARTNUM(x)		(((x) >> 4) & 0xfff)
#define	MIDR_VARIANT(x)		(((x) >> 20) & 0xf)
#define	MIDR_IMPLEMENTER(x)	(((x) >> 24) & 0xff)
#define	CPU_IMPL_ARM		0x41
#define	CPU_IMPL_BROADCOM	0x42
#define	CPU_IMPL_CAVIUM		0x43
#define	CPU_IMPL_QUALCOMM	0x51
#define	CPU_IMPL_APPLE		0x61
#define	CPU_IMPL_AMPERE		0xc0

/* MPIDR_EL1 */
#define	MPIDR_AFF_MASK		0x000000ff00ffffffUL

/* ID_AA64MMFR0_EL1 */
#define	ID_AA64MMFR0_PARANGE(x)	((x) & 0xf)
#define	ID_AA64MMFR0_ASIDBITS(x) (((x) >> 4) & 0xf)
#define	ID_AA64MMFR0_TGRAN4(x)	(((x) >> 28) & 0xf)

/* ID_AA64PFR0_EL1 */
#define	ID_AA64PFR0_EL2(x)	(((x) >> 8) & 0xf)
#define	ID_AA64PFR0_EL3(x)	(((x) >> 12) & 0xf)
#define	ID_AA64PFR0_FP(x)	(((x) >> 16) & 0xf)
#define	ID_AA64PFR0_ADVSIMD(x)	(((x) >> 20) & 0xf)
#define	ID_AA64PFR0_GIC(x)	(((x) >> 24) & 0xf)
#define	ID_AA64PFR0_SVE(x)	(((x) >> 32) & 0xf)

/* ID_AA64ISAR0_EL1 */
#define	ID_AA64ISAR0_AES(x)	(((x) >> 4) & 0xf)
#define	ID_AA64ISAR0_SHA2(x)	(((x) >> 12) & 0xf)
#define	ID_AA64ISAR0_CRC32(x)	(((x) >> 16) & 0xf)
#define	ID_AA64ISAR0_ATOMIC(x)	(((x) >> 20) & 0xf)	/* LSE */
#define	ID_AA64ISAR0_RNDR(x)	(((x) >> 60) & 0xf)

/* Generic timer control (CNTV_CTL_EL0 / CNTP_CTL_EL0) */
#define	CNTx_CTL_ENABLE		0x1
#define	CNTx_CTL_IMASK		0x2
#define	CNTx_CTL_ISTATUS	0x4

#endif /* !_CPU_ARMREG_H_ */
