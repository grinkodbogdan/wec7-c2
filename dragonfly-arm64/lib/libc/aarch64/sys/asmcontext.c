/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 * BSD 2-clause; see COPYRIGHT.
 */
#define _KERNEL_STRUCTURES
#include <sys/types.h>
#include <sys/signal.h>
#include <sys/ucontext.h>
#include <sys/assym.h>
#include <stddef.h>

ASSYM(UC_SIGMASK, offsetof(ucontext_t, uc_sigmask));
ASSYM(UC_LINK, offsetof(ucontext_t, uc_link));
ASSYM(UC_MCONTEXT, offsetof(ucontext_t, uc_mcontext));
ASSYM(SIG_BLOCK, SIG_BLOCK);
ASSYM(MC_ONSTACK, offsetof(mcontext_t, mc_onstack));
ASSYM(MC_SP, offsetof(mcontext_t, mc_sp));
ASSYM(MC_LR, offsetof(mcontext_t, mc_lr));
ASSYM(MC_ELR, offsetof(mcontext_t, mc_elr));
ASSYM(MC_SPSR, offsetof(mcontext_t, mc_spsr));
ASSYM(MC_X, offsetof(mcontext_t, mc_x));
ASSYM(MC_TPIDR, offsetof(mcontext_t, mc_tpidr));
ASSYM(MC_FPREGS, offsetof(mcontext_t, mc_fpregs));
ASSYM(MC_LEN, offsetof(mcontext_t, mc_len));
ASSYM(MC_FPFORMAT, offsetof(mcontext_t, mc_fpformat));
ASSYM(MC_OWNEDFP, offsetof(mcontext_t, mc_ownedfp));
ASSYM(SIZEOF_MCONTEXT_T, sizeof(mcontext_t));
ASSYM(_MC_FPOWNED_FPU, _MC_FPOWNED_FPU);
ASSYM(_MC_FPFMT_VFP, _MC_FPFMT_VFP);
