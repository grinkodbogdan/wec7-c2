# bootstub — M4 Proof-of-Life

The **smallest atomic boot-stage artifact** for WEC7-C2 (engineering Rule 6): a
bare-metal ARMv7-A program that brings CPU0 to a known state and emits a deterministic
banner + heartbeat over the debug UART.

This is the concrete "make something boot" primitive. It is **not** the WEC7 kernel — it
is the minimal executable that proves the three things every later milestone depends on
(Rule 10, diagnostics before UI):

1. the bootloader hands off and our entry actually runs,
2. CPU + stack init is correct (SVC mode, IRQ/FIQ masked, secondaries parked),
3. the **debug channel works**, so all subsequent bring-up is observable.

## Files

| File | Purpose |
|------|---------|
| `start.S` | ARMv7-A entry: MPIDR core guard, CPSR/stack setup, `.bss` clear, UART init, banner + heartbeat. |
| `bootstub.ld` | Linker script. Load address is a **placeholder** (HW map §2/§13). |
| `build.sh` | Reproducible build → `build/bootstub.{elf,bin,lst}` (Rule 7). |

## Build

```sh
# Needs an arm-none-eabi bare-metal toolchain:
#   apt-get install -y gcc-arm-none-eabi binutils-arm-none-eabi
./build.sh
```

Outputs:
- `build/bootstub.elf` — linked ELF (inspect with gdb/objdump).
- `build/bootstub.bin` — raw image a bootloader would load.
- `build/bootstub.lst` — disassembly, for auditing the emitted ARMv7-A.

Build outputs are git-ignored. This step validates the code **assembles and links to
real ARMv7-A** — it is the "buildability" half of the milestone. **Building is not
booting.**

## What is proven here vs. what is NOT

**Proven (in this repo, today):**
- The startup/UART logic assembles and links cleanly for `cortex-a7` / ARMv7-A.
- The disassembly matches intent: `_start` first at the load address, MPIDR guard,
  SVC-mode entry, MSM UART_DM TX path (poll `SR` → set `NCF_TX` → write `TF`).

**NOT proven (blocked — do not skip):**
- That it runs on a C2. Three hard gates first:
  1. **Debug UART base is `VERIFY`** (HW map §6). `MSM_UART_BASE = 0x078B0000` is a
     *family* candidate, not confirmed against the skipjack device tree. A wrong base =
     silent boot.
  2. **Bootloader handoff contract is `VERIFY`** (HW map §13.1). Whether C2 `aboot` will
     load/jump to a raw image, at what address, in what CPU state, is unknown.
  3. **Reserved-memory carveouts are `VERIFY`** (HW map §2). The load address and stack
     must avoid TrustZone/modem regions.
- Safety gates from the spec still apply: M0 backups verified (Rule 2), volatile
  `fastboot boot` only — never `fastboot flash` an unverified image (Rule 4).

## How this becomes a real on-device boot (ordered)

1. **M1:** promote HW map §6 (UART base), §13.1 (handoff), §2 (carveouts) to `CONFIRMED`
   from the AsteroidOS `skipjack` dts + live Void Linux dumps.
2. Fix `MSM_UART_BASE`, `STACK_TOP`, and the linker load address to the confirmed values.
3. **M0:** complete and verify partition backups.
4. Wrap `bootstub.bin` in whatever container the bootloader requires (M2 analysis) and
   `fastboot boot` it — volatile, never flashed.
5. Observe the banner on the confirmed debug channel → **M4 proof-of-life achieved.**
6. Grow this stub into the OAL early-debug module (`bsp/C2_SKIPJACK/Src/Oal/debug/`),
   then hand off to the WEC7 kernel entry (`bsp/.../Src/Startup/`).

## Dev-time smoke test (optional, not a project target)

The spec excludes QEMU as a *port target*. For pure logic testing of the UART state
machine you could port the base address to a QEMU `virt` PL011 and run it under
`qemu-system-arm` — but that tests our code, not the C2, and the MSM UART_DM protocol
differs from PL011, so it is of limited value. On-hardware serial is the real test.
