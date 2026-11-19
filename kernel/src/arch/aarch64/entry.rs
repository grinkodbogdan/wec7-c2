//! Boot entry, exception vector table and the user-entry trampoline.
//!
//! Trap frame layout (272 bytes, see `TrapFrame`):
//!   x0..x30 at 0..240, sp_el0 at 248, elr_el1 at 256, spsr_el1 at 264.

use core::arch::global_asm;

global_asm!(
    r#"
.section .text.boot, "ax"
.global _start
_start:
    // Runs at the physical load address with the MMU off: PC-relative code only.
    // Park secondary CPUs (SMP comes later).
    mrs     x1, mpidr_el1
    and     x1, x1, #0xff
    cbz     x1, 2f
1:  wfe
    b       1b
2:
    // If entered at EL2, drop to EL1h with all interrupts masked.
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.ne    3f
    mov     x0, #(1 << 31)          // HCR_EL2.RW: EL1 is AArch64
    msr     hcr_el2, x0
    mov     x0, #3                  // EL1 access to the physical counter/timer
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr
    mov     x0, #0x3c5              // EL1h, DAIF masked
    msr     spsr_el2, x0
    adr     x0, 3f
    msr     elr_el2, x0
    eret
3:
    msr     daifset, #0xf
    // Boot translation tables: map the 1 GiB holding the kernel both at its
    // physical address (TTBR0, to survive turning the MMU on) and at its
    // upper-half alias (TTBR1). kmain replaces both.
    adrp    x0, __boot_l1_lo
    adrp    x1, __boot_l1_hi
    mov     x2, #512
4:  str     xzr, [x0], #8
    str     xzr, [x1], #8
    subs    x2, x2, #1
    b.ne    4b
    adrp    x0, __boot_l1_lo
    adrp    x1, __boot_l1_hi
    adrp    x2, _start
    lsr     x3, x2, #30             // L1 index of the kernel's gigabyte
    lsl     x4, x3, #30
    mov     x5, #0x705              // block | Normal (attr 1) | inner shareable | AF
    orr     x4, x4, x5
    str     x4, [x0, x3, lsl #3]
    str     x4, [x1, x3, lsl #3]
    ldr     x2, =0xff00             // MAIR: attr0 Device-nGnRnE, attr1 Normal WB
    msr     mair_el1, x2
    ldr     x2, =((25) | (1 << 8) | (1 << 10) | (3 << 12) | (25 << 16) | (1 << 24) | (1 << 26) | (3 << 28) | (2 << 30) | (2 << 32))
    msr     tcr_el1, x2
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x1
    dsb     ish
    tlbi    vmalle1
    dsb     ish
    isb
    mrs     x2, sctlr_el1
    orr     x2, x2, #(1 << 0)       // M
    orr     x2, x2, #(1 << 2)       // C
    orr     x2, x2, #(1 << 12)      // I
    bic     x2, x2, #(1 << 1)       // A
    bic     x2, x2, #(1 << 19)      // WXN
    msr     sctlr_el1, x2
    isb
    ldr     x0, =__high_entry
    br      x0
__high_entry:
    ldr     x0, =__stack_top
    mov     sp, x0
    ldr     x0, =__bss_start
    ldr     x1, =__bss_end
5:  cmp     x0, x1
    b.hs    6f
    str     xzr, [x0], #8
    b       5b
6:
    bl      kmain
7:  wfe
    b       7b

.section .text.vectors, "ax"
.macro VECTOR kind
    .balign 0x80
    sub     sp, sp, #272
    stp     x0, x1, [sp, #0]
    mov     x1, #\kind
    b       __exception_save
.endm

.balign 2048
.global __vectors
__vectors:
    VECTOR 0
    VECTOR 1
    VECTOR 2
    VECTOR 3
    VECTOR 4
    VECTOR 5
    VECTOR 6
    VECTOR 7
    VECTOR 8
    VECTOR 9
    VECTOR 10
    VECTOR 11
    VECTOR 12
    VECTOR 13
    VECTOR 14
    VECTOR 15

__exception_save:
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    mrs     x21, sp_el0
    stp     x30, x21, [sp, #240]
    mrs     x22, elr_el1
    mrs     x23, spsr_el1
    stp     x22, x23, [sp, #256]
    mov     x0, sp
    bl      exception_entry
    b       __exception_restore

__exception_restore:
    ldp     x22, x23, [sp, #256]
    msr     elr_el1, x22
    msr     spsr_el1, x23
    ldp     x30, x21, [sp, #240]
    msr     sp_el0, x21
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    add     sp, sp, #272
    eret

// __enter_first(frame: *const TrapFrame): copy the frame to the top of the
// kernel stack and return to user space through the common restore path.
.global __enter_first
__enter_first:
    ldr     x1, =__stack_top
    sub     x1, x1, #272
    mov     x2, #34
8:  ldr     x3, [x0], #8
    str     x3, [x1], #8
    subs    x2, x2, #1
    b.ne    8b
    ldr     x1, =__stack_top
    sub     sp, x1, #272
    b       __exception_restore
"#
);
