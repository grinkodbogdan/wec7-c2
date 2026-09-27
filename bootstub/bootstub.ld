/*
 * Linker script for the WEC7-C2 bare-metal boot stub (M4 proof-of-life).
 *
 * LOAD ADDRESS IS A PLACEHOLDER. The real load/link address is dictated by the
 * bootloader handoff contract (docs/01-hardware-map.md §13.1, currently VERIFY)
 * and must sit in a DDR region confirmed clear of TrustZone/modem carveouts
 * (HW map §2). Do not treat 0x80080000 as final — it is a plausible Qualcomm
 * DDR-base + 512K offset used only so the object links and can be inspected.
 */

OUTPUT_FORMAT("elf32-littlearm")
OUTPUT_ARCH(arm)
ENTRY(_start)

/* HW map §2: DDR base 0x80000000. Kernel/loadables typically placed above the
 * low reserved regions; 0x80080000 chosen as a stub placeholder only. */
_LOAD_ADDR = 0x80080000;

MEMORY
{
    RAM (rwx) : ORIGIN = 0x80080000, LENGTH = 16M   /* placeholder window */
}

SECTIONS
{
    . = _LOAD_ADDR;

    .text : {
        KEEP(*(.text.boot))     /* _start must come first */
        *(.text*)
        *(.rodata*)
    } > RAM

    .data : ALIGN(8) {
        *(.data*)
    } > RAM

    .bss (NOLOAD) : ALIGN(8) {
        __bss_start__ = .;
        *(.bss*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end__ = .;
    } > RAM

    _end = .;

    /DISCARD/ : { *(.ARM.exidx* .comment .note*) }
}
