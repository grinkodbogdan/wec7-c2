/*
 * WEC7-C2 bare-metal boot stub — M4 proof-of-life.
 *
 * Smallest atomic boot-stage artifact (engineering Rule 6): bring the CPU to a
 * known ARMv7-A state, set up a stack, and emit a deterministic banner over the
 * debug UART. This is the "make something boot" primitive — before any WEC7
 * kernel, prove the handoff → CPU-init → UART-out path (Rule 10: diagnostics
 * before UI).
 *
 * This is NOT the WEC7 kernel. It is the minimal executable that validates:
 *   1. the bootloader hands off and our entry runs at all,
 *   2. CPU/stack init is correct,
 *   3. the debug channel works so all later bring-up is observable.
 *
 * Target: Qualcomm MSM8909W (Snapdragon Wear 2100), 4x Cortex-A7, ARMv7-A.
 *
 * !!! CONFIDENCE: the UART base below is FAMILY-level (MSM8909 family), NOT yet
 * CONFIRMED against the skipjack device tree. See docs/01-hardware-map.md §6.
 * Per Rule 8 the magic address is recorded there and here with its source and
 * confidence. Do NOT run this on hardware until the base is CONFIRMED and the
 * bootloader handoff contract (HW map §13.1) is understood (Rules 2, 4).
 */

    .arch   armv7-a
    .syntax unified
    .arm

/* ------------------------------------------------------------------------- */
/* Board constants — every value here MUST trace to docs/01-hardware-map.md.   */
/* ------------------------------------------------------------------------- */

/* Debug UART: Qualcomm BLSP QUP UART (msm_serial, DM mode). HW map §6.
 * Confidence: FAMILY / VERIFY. Candidate is BLSP1_UART2 on the MSM8909 family.
 * MSM_UART_DM register offsets (data-mover mode): */
    .equ MSM_UART_BASE,       0x078B0000   /* HW map §6 — VERIFY vs skipjack dts */
    .equ UARTDM_SR,           0x00A4       /* status register        */
    .equ UARTDM_CR,           0x00A8       /* command register       */
    .equ UARTDM_NCF_TX,       0x0040       /* # chars for TX         */
    .equ UARTDM_TF,           0x0100       /* TX FIFO (packed 4B)    */
    .equ UART_SR_TXRDY,       (1 << 2)     /* TX ready               */
    .equ UART_SR_TXEMT,       (1 << 3)     /* TX empty               */
    .equ UART_CR_CMD_RESET_TX_READY, 0x300 /* "reset TX ready" cmd   */

/* Initial stack top. Placeholder — must land in a RAM region confirmed free of
 * TrustZone/modem carveouts before on-device use. HW map §2 (DDR base
 * 0x80000000, 512 MB). Using base+16MB purely as a stub-local scratch. */
    .equ STACK_TOP,           0x81000000   /* HW map §2 — VERIFY carveouts */

/* ------------------------------------------------------------------------- */
/* Vector / entry.                                                            */
/* ------------------------------------------------------------------------- */
    .section .text.boot, "ax"
    .global _start
_start:
    /*
     * Entry state is defined by the bootloader handoff contract (HW map §13.1),
     * which is currently VERIFY. We assume nothing and force a known state.
     */

    /* --- Multi-core guard: park all but CPU0 (MPIDR affinity 0). --------- */
    mrc     p15, 0, r0, c0, c0, 5      /* read MPIDR                        */
    and     r0, r0, #0xFF              /* Aff0 = core id within cluster     */
    cmp     r0, #0
    bne     park_secondary            /* secondaries spin (WFE loop)       */

    /* --- CPSR: SVC mode, IRQ/FIQ masked, ARM state. --------------------- */
    cpsid   if                        /* mask IRQ + FIQ                    */
    mrs     r0, cpsr
    bic     r0, r0, #0x1F             /* clear mode bits                   */
    orr     r0, r0, #0x13            /* SVC (0b10011)                     */
    msr     cpsr_c, r0

    /* --- Stack. --------------------------------------------------------- */
    ldr     sp, =STACK_TOP

    /* --- Invalidate/prep: keep it minimal (MMU/caches left off for the    */
    /*     stub; a real OAL enables them after the memory map is CONFIRMED).*/

    /* --- Zero the .bss so C globals are sane if we grow into C. --------- */
    ldr     r0, =__bss_start__
    ldr     r1, =__bss_end__
    mov     r2, #0
1:  cmp     r0, r1
    bge     2f
    str     r2, [r0], #4
    b       1b
2:

    /* --- Bring up debug UART + emit banner (the proof-of-life). --------- */
    bl      uart_init
    ldr     r0, =banner
    bl      uart_puts

    /* --- Heartbeat: emit a dot forever so a live core is observable. ---- */
hb_loop:
    mov     r0, #'.'
    bl      uart_putc
    ldr     r0, =0x00800000          /* crude delay count                 */
3:  subs    r0, r0, #1
    bne     3b
    b       hb_loop

park_secondary:
    wfe
    b       park_secondary

/* ------------------------------------------------------------------------- */
/* UART primitives — MSM UART_DM (data-mover) minimal TX path.                 */
/*   NOTE: DM-mode TX requires programming NO_CHARS_FOR_TX then writing packed */
/*   words to TF. This is a minimal, one-char-at-a-time implementation meant   */
/*   for early bring-up clarity, not throughput.                               */
/* ------------------------------------------------------------------------- */

    .global uart_init
uart_init:
    /* The bootloader/Linux typically left the UART clocked and configured.
     * Under WEC7 the OAL clock module (GCC, HW map §5) must ungate the BLSP
     * UART clock first — that is a VERIFY step and lives in the OAL, not here.
     * For this stub we assume the debug UART is already clocked (as when
     * chain-loaded after the bootloader) and only reset the TX path. */
    ldr     r3, =MSM_UART_BASE
    ldr     r0, =UART_CR_CMD_RESET_TX_READY
    str     r0, [r3, #UARTDM_CR]
    bx      lr

    .global uart_putc
    /* r0 = byte to send. Clobbers r1-r3. */
uart_putc:
    ldr     r3, =MSM_UART_BASE
    /* wait for TX ready */
1:  ldr     r1, [r3, #UARTDM_SR]
    tst     r1, #UART_SR_TXRDY
    beq     1b
    /* program 1 char for this transfer */
    mov     r1, #1
    str     r1, [r3, #UARTDM_NCF_TX]
    /* write the byte (low 8 bits of the packed TX word) */
    and     r0, r0, #0xFF
    str     r0, [r3, #UARTDM_TF]
    bx      lr

    .global uart_puts
    /* r0 = ptr to NUL-terminated string. Clobbers r0-r4. */
uart_puts:
    mov     r4, r0
1:  ldrb    r0, [r4], #1
    cmp     r0, #0
    bxeq    lr
    /* translate LF -> CRLF for terminal friendliness */
    cmp     r0, #'\n'
    bne     2f
    mov     r0, #'\r'
    push    {r4, lr}
    bl      uart_putc
    pop     {r4, lr}
    mov     r0, #'\n'
2:  push    {r4, lr}
    bl      uart_putc
    pop     {r4, lr}
    b       1b

/* ------------------------------------------------------------------------- */
    .section .rodata
banner:
    .asciz  "\n=== WEC7-C2 bootstub alive: CPU0 up, UART live (skipjack/MSM8909W) ===\n"

    .end
