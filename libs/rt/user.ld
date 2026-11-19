/* Link script for native Flare components.
 * Images live at flare_abi::layout::USER_IMAGE_BASE. Code, read-only data
 * and writable data get separate segments so the kernel can map them
 * R-X, R-- and RW- (W^X). */
ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);   /* R X */
    rodata PT_LOAD FLAGS(4);   /* R   */
    data   PT_LOAD FLAGS(6);   /* RW  */
}

SECTIONS
{
    . = 0x200000000;
    .text : {
        KEEP(*(.text.start))
        *(.text .text.*)
    } :text

    . = ALIGN(4096);
    .rodata : {
        *(.rodata .rodata.*)
        *(.got .got.*)
    } :rodata

    . = ALIGN(4096);
    .data : {
        *(.data .data.*)
    } :data
    .bss (NOLOAD) : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
    }
}
