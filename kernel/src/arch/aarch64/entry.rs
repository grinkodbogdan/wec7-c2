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
    // The Linux arm64 Image header, so boot loaders that boot Linux (U-Boot,
    // Qualcomm's LK, QEMU's -kernel) load the kernel as they would Linux: at
    // DRAM base + text_offset, entered at the first instruction with x0 =
    // their device tree (unused: the board is described by FHD).
    b       __boot                  // code0
    .long   0                       // code1
    .quad   0x80000                 // text_offset
    .quad   __image_size            // image_size, including .bss
    .quad   0x2                     // flags: little-endian, 4 KiB pages, placed at DRAM base + text_offset
    .quad   0, 0, 0                 // res2..res4
    .ascii  "ARM\x64"               // magic
    .long   0                       // res5: no PE/COFF header
__boot:
    // Runs at the physical load address with the MMU off: PC-relative code only.
    // Park secondary CPUs (SMP comes later).
    mrs     x1, mpidr_el1
    and     x1, x1, #0xff
    cbz     x1, 2f
1:  wfe
    b       1b
2:
    // Boot mark 0: the boot loader entered the kernel (a white stripe at the
    // top of a framebuffer console; see console::early_mark).
    mov     x0, #0
    bl      __early_stripe
    // Run where the kernel was linked. Boot loaders differ (Qualcomm's LK
    // can use the boot image's address or its own): if this is not the
    // linked physical address, move the image there (memmove; the MMU and
    // data cache are off) and continue at the copy.
    adr     x0, _start
    ldr     x1, =_start_phys
    cmp     x0, x1
    b.eq    13f
    ldr     x2, =__load_size
    b.hi    14f
    // dest above src: copy from the top down (never reaches this code)
    add     x0, x0, x2
    add     x1, x1, x2
10: ldr     x3, [x0, #-8]!
    str     x3, [x1, #-8]!
    subs    x2, x2, #8
    b.hi    10b
    b       12f
    // dest below src: an ascending copy would overwrite this code before
    // it finished. Copy the first 4 KiB (this code) down first, then run
    // the whole copy from there: it rewrites that page with the same bytes.
14: sub     x5, x0, x1
    cmp     x5, #4096
    b.lo    1b                      // too close to move safely: stop
    mov     x5, #0
15: ldr     x3, [x0, x5]
    str     x3, [x1, x5]
    add     x5, x5, #8
    cmp     x5, #4096
    b.lo    15b
    dsb     sy
    ic      iallu
    dsb     sy
    isb
    adr     x5, 11f
    sub     x5, x5, x0
    add     x5, x5, x1
    br      x5
11: ldr     x3, [x0], #8
    str     x3, [x1], #8
    subs    x2, x2, #8
    b.hi    11b
12: dsb     sy
    ic      iallu
    dsb     sy
    isb
    ldr     x0, =(__boot - 0xffffff8000000000)
    br      x0
13:
    mov     x0, #1                  // boot mark 1: at the linked address
    bl      __early_stripe
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
    // Boot translation tables: map the kernel image, in 2 MiB blocks, both
    // at its physical address (TTBR0, to survive turning the MMU on) and at
    // its upper-half alias (TTBR1); kmain replaces both. Nothing else is
    // mapped: on phone SoCs, firmware memory next to the kernel must not be
    // reachable even speculatively.
    adrp    x0, __boot_l1_lo
    adrp    x1, __boot_l1_hi
    adrp    x6, __boot_l2
    mov     x2, #512
4:  str     xzr, [x0], #8
    str     xzr, [x1], #8
    str     xzr, [x6], #8
    subs    x2, x2, #1
    b.ne    4b
    adrp    x0, __boot_l1_lo
    adrp    x1, __boot_l1_hi
    adrp    x6, __boot_l2
    adrp    x2, _start
    lsr     x3, x2, #30             // L1 index of the kernel's gigabyte
    orr     x4, x6, #3              // table descriptor -> __boot_l2
    str     x4, [x0, x3, lsl #3]
    str     x4, [x1, x3, lsl #3]
    lsr     x7, x2, #21             // first 2 MiB block of the image
    lsl     x7, x7, #21
    adrp    x8, __kernel_end
    mov     x5, #0x705              // block | Normal (attr 1) | inner shareable | AF
40: lsr     x9, x7, #21
    and     x9, x9, #0x1ff
    orr     x10, x7, x5
    str     x10, [x6, x9, lsl #3]
    add     x7, x7, #0x200000
    cmp     x7, x8
    b.lo    40b
    // The tables were written with the data cache off: drop any stale
    // lines a boot loader left for them, so the walker and later cached
    // accesses see what is in memory (as Linux's head.S does).
    adrp    x2, __boot_l1_lo
    add     x3, x2, #(3 * 4096)
41: dc      ivac, x2
    add     x2, x2, #64
    cmp     x2, x3
    b.lo    41b
    dsb     sy
    ldr     x2, =0x44ff00           // MAIR: attr0 Device-nGnRnE, attr1 Normal WB, attr2 Normal NC
    msr     mair_el1, x2
    ldr     x2, =((25) | (1 << 8) | (1 << 10) | (3 << 12) | (25 << 16) | (1 << 24) | (1 << 26) | (3 << 28) | (2 << 30) | (2 << 32))
    msr     tcr_el1, x2
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x1
    dsb     ish
    tlbi    vmalle1
    dsb     ish
    isb
    // SCTLR_EL1 from scratch, not from what the boot loader left (little
    // endian, no alignment checks, no WXN): RES1 bits | M | C | I | DZE |
    // UCT | UCI.
    ldr     x2, =(0x30d00800 | (1 << 0) | (1 << 2) | (1 << 12) | (1 << 14) | (1 << 15) | (1 << 26))
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

// Boot mark x0 with the MMU off: fill rows [x0 * 48, x0 * 48 + 32) of the
// framebuffer console (EARLY_FB: physical base, bytes per row, rows; base 0
// for none) with white. Uses x0-x7 only and no stack.
__early_stripe:
    adrp    x1, EARLY_FB
    add     x1, x1, :lo12:EARLY_FB
    ldr     x2, [x1]
    cbz     x2, 2f
    ldr     x3, [x1, #8]
    mov     x4, #48
    mul     x4, x4, x0
    mul     x4, x4, x3
    add     x2, x2, x4
    mov     x5, #32
    mul     x5, x5, x3
    mov     x6, #-1
1:  str     x6, [x2], #8
    subs    x5, x5, #8
    b.hi    1b
    dsb     sy
2:  ret

// Secondary CPUs, started by PSCI CPU_ON at their physical address with the
// MMU off; x0 = CPU index. Same tables as the boot CPU, then the upper half.
.global __secondary_entry
__secondary_entry:
    msr     daifset, #0xf
    mov     x19, x0
    adrp    x0, __boot_l1_lo
    adrp    x1, __boot_l1_hi
    ldr     x2, =0x44ff00
    msr     mair_el1, x2
    ldr     x2, =((25) | (1 << 8) | (1 << 10) | (3 << 12) | (25 << 16) | (1 << 24) | (1 << 26) | (3 << 28) | (2 << 30) | (2 << 32))
    msr     tcr_el1, x2
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x1
    dsb     ish
    tlbi    vmalle1
    dsb     ish
    isb
    ldr     x2, =(0x30d00800 | (1 << 0) | (1 << 2) | (1 << 12) | (1 << 14) | (1 << 15) | (1 << 26))
    msr     sctlr_el1, x2
    isb
    ldr     x0, =__secondary_high
    br      x0
__secondary_high:
    ldr     x1, =SECONDARY_STACKS
    add     x2, x19, #1
    lsl     x2, x2, #16                 // (id + 1) * 64 KiB
    add     sp, x1, x2
    mov     x0, x19
    bl      secondary_main
9:  wfe
    b       9b

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

// __enter_first(frame: *const TrapFrame, stack_top): copy the frame to the
// top of this CPU's kernel stack and return to user space through the
// common restore path.
.global __enter_first
__enter_first:
    mov     x4, x1
    sub     x1, x1, #272
    mov     x2, #34
8:  ldr     x3, [x0], #8
    str     x3, [x1], #8
    subs    x2, x2, #1
    b.ne    8b
    sub     sp, x4, #272
    b       __exception_restore
"#
);
