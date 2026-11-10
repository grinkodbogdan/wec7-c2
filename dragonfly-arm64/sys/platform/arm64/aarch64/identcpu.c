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

/*
 * CPU identification from the aarch64 ID registers.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/sysctl.h>
#include <cpu/armreg.h>
#include <machine/md_var.h>

struct cpu_part {
	u_int		impl;
	u_int		part;
	const char	*name;
};

static const struct cpu_part cpu_parts[] = {
	{ CPU_IMPL_ARM,		0xd03,	"Cortex-A53" },
	{ CPU_IMPL_ARM,		0xd04,	"Cortex-A35" },
	{ CPU_IMPL_ARM,		0xd05,	"Cortex-A55" },
	{ CPU_IMPL_ARM,		0xd07,	"Cortex-A57" },
	{ CPU_IMPL_ARM,		0xd08,	"Cortex-A72" },
	{ CPU_IMPL_ARM,		0xd09,	"Cortex-A73" },
	{ CPU_IMPL_ARM,		0xd0a,	"Cortex-A75" },
	{ CPU_IMPL_ARM,		0xd0b,	"Cortex-A76" },
	{ CPU_IMPL_ARM,		0xd0c,	"Neoverse-N1" },
	{ CPU_IMPL_ARM,		0xd0d,	"Cortex-A77" },
	{ CPU_IMPL_ARM,		0xd40,	"Neoverse-V1" },
	{ CPU_IMPL_ARM,		0xd41,	"Cortex-A78" },
	{ CPU_IMPL_ARM,		0xd44,	"Cortex-X1" },
	{ CPU_IMPL_ARM,		0xd46,	"Cortex-A510" },
	{ CPU_IMPL_ARM,		0xd47,	"Cortex-A710" },
	{ CPU_IMPL_ARM,		0xd48,	"Cortex-X2" },
	{ CPU_IMPL_ARM,		0xd49,	"Neoverse-N2" },
	{ CPU_IMPL_QUALCOMM,	0x800,	"Kryo 2xx Gold" },
	{ CPU_IMPL_QUALCOMM,	0x801,	"Kryo 2xx Silver" },
	{ CPU_IMPL_QUALCOMM,	0x802,	"Kryo 3xx Gold" },
	{ CPU_IMPL_QUALCOMM,	0x803,	"Kryo 3xx Silver" },
	{ CPU_IMPL_QUALCOMM,	0x804,	"Kryo 4xx Gold" },
	{ CPU_IMPL_QUALCOMM,	0x805,	"Kryo 4xx Silver" },
	{ CPU_IMPL_APPLE,	0x022,	"Icestorm (M1)" },
	{ CPU_IMPL_APPLE,	0x023,	"Firestorm (M1)" },
	{ 0, 0, NULL }
};

static const char *
cpu_impl_name(u_int impl)
{
	switch (impl) {
	case CPU_IMPL_ARM:	return ("ARM");
	case CPU_IMPL_BROADCOM:	return ("Broadcom");
	case CPU_IMPL_CAVIUM:	return ("Cavium");
	case CPU_IMPL_QUALCOMM:	return ("Qualcomm");
	case CPU_IMPL_APPLE:	return ("Apple");
	case CPU_IMPL_AMPERE:	return ("Ampere");
	default:		return ("Unknown vendor");
	}
}

char	machine[] = MACHINE;
SYSCTL_STRING(_hw, HW_MACHINE, machine, CTLFLAG_RD,
    machine, 0, "Machine class");

static char cpu_model[64] = "unknown";
SYSCTL_STRING(_hw, HW_MODEL, model, CTLFLAG_RD,
    cpu_model, 0, "Machine model");

void
identify_cpu(void)
{
	static const u_char parange_bits[] = { 32, 36, 40, 42, 44, 48, 52 };
	uint64_t midr = READ_SPECIALREG(midr_el1);
	uint64_t mmfr0 = READ_SPECIALREG(id_aa64mmfr0_el1);
	uint64_t pfr0 = READ_SPECIALREG(id_aa64pfr0_el1);
	uint64_t isar0 = READ_SPECIALREG(id_aa64isar0_el1);
	const struct cpu_part *cp;
	const char *name = "unknown core";
	u_int pa;

	for (cp = cpu_parts; cp->name != NULL; ++cp) {
		if (cp->impl == MIDR_IMPLEMENTER(midr) &&
		    cp->part == MIDR_PARTNUM(midr)) {
			name = cp->name;
			break;
		}
	}
	ksnprintf(cpu_model, sizeof(cpu_model), "%s %s",
	    cpu_impl_name(MIDR_IMPLEMENTER(midr)), name);
	kprintf("CPU: %s %s r%lup%lu (midr 0x%08lx)\n",
	    cpu_impl_name(MIDR_IMPLEMENTER(midr)), name,
	    MIDR_VARIANT(midr), MIDR_REVISION(midr), midr);

	pa = ID_AA64MMFR0_PARANGE(mmfr0);
	kprintf("  Physical address bits: %u, ASID bits: %u, running at EL%lu\n",
	    pa < sizeof(parange_bits) ? parange_bits[pa] : 0,
	    ID_AA64MMFR0_ASIDBITS(mmfr0) == 2 ? 16 : 8,
	    CURRENTEL_EL(READ_SPECIALREG(CurrentEL)));

	kprintf("  Features:");
	if (ID_AA64PFR0_FP(pfr0) != 0xf)
		kprintf(" FP");
	if (ID_AA64PFR0_ADVSIMD(pfr0) != 0xf)
		kprintf(" AdvSIMD");
	if (ID_AA64PFR0_SVE(pfr0) != 0)
		kprintf(" SVE");
	if (ID_AA64ISAR0_AES(isar0) != 0)
		kprintf(" AES");
	if (ID_AA64ISAR0_SHA2(isar0) != 0)
		kprintf(" SHA2");
	if (ID_AA64ISAR0_CRC32(isar0) != 0)
		kprintf(" CRC32");
	if (ID_AA64ISAR0_ATOMIC(isar0) != 0)
		kprintf(" LSE");
	if (ID_AA64ISAR0_RNDR(isar0) != 0)
		kprintf(" RNDR");
	if (ID_AA64PFR0_EL2(pfr0) != 0)
		kprintf(" EL2");
	if (ID_AA64PFR0_EL3(pfr0) != 0)
		kprintf(" EL3");
	kprintf("\n");
}
