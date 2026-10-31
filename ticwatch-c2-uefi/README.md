# EDK2 / UEFI for Mobvoi TicWatch C2 (`skipjack`, MSM8909W)

A debug-first roadmap and scaffolding for porting **EDK2/UEFI (32-bit AArch32)** to
the Mobvoi TicWatch C2.

> **Scope decision:** UEFI only. Windows 10/11 on ARM is **not** a target on this device
> and cannot be, because the Snapdragon Wear 2100 (MSM8909W) is a **Cortex-A7, ARMv7-A,
> 32-bit** part. Windows 10/11 on ARM is AArch64-only. The deliverable here is a working
> UEFI firmware with on-device debug output (screen / GPIO blink / UART).

> **Status:** Documentation + debug scaffolding. Not a flashable build. The actual port
> is iterative work on physical hardware and cannot be completed in a CI container.

## Device facts (from the project spec; verify against your unit)

| Property        | Value                                                          |
|-----------------|----------------------------------------------------------------|
| Device          | Mobvoi TicWatch C2                                              |
| Codename        | `skipjack`                                                     |
| SoC             | Qualcomm Snapdragon Wear 2100 = **MSM8909W**                   |
| CPU             | Quad **Cortex-A7**, **ARMv7-A (32-bit)**                        |
| RAM             | 512 MB                                                          |
| Display         | ~360 × 360, round panel                                        |
| Bootloader      | Unlocked (fastboot available)                                  |
| Known-good OSes | AsteroidOS + Void Linux (mainline-style Linux, `skipjack` DTS) |

## Why EDK2 here is realistic (and where it's hard)

- **EDK2 supports AArch32.** You build with `-a ARM` (the 32-bit ARM arch), not `AARCH64`.
  So a 32-bit UEFI on a Cortex-A7 is a legitimate build target.
- **You have a Linux device tree.** Because the C2 already runs AsteroidOS/Void Linux,
  the `skipjack` `.dts` and a live `dmesg`/`/proc`/`/sys` give you the exact register
  map, clock tree, GPIO assignments, and framebuffer geometry. This is a *huge*
  advantage — most phone UEFI ports don't start with a clean mainline DTS.
- **The hard part:** there is no off-the-shelf MSM8909W UEFI package for a watch. You
  adapt the generic EDK2 ARM platform support plus a minimal MSM8909 platform package,
  and you hand-port the handful of SoC blocks UEFI needs (see `docs/02`).

## The one rule that matters most

**Get a debug signal off the device before you change anything else.** On a watch there
are three channels, in rough order of ease:

1. **Screen** — paint the 360×360 framebuffer the bootloader already lit up. See `docs/03`.
2. **GPIO "blink"** — toggle the vibration motor / backlight / a test-point GPIO as POST
   codes (blink patterns). The watch has no classic notification LED, so a GPIO you can
   feel/see is the substitute. See `docs/03` + `debug/post_codes.h`.
3. **UART** — on test points; most reliable once found. See `docs/03`.

## Non-negotiable rules (carried from the project spec)

1. Preserve the working AsteroidOS / Void Linux installs.
2. Raw partition backups **before** any boot change; verify fastboot (and EDL/9008)
   restore paths first. An unlocked bootloader does **not** guarantee un-brick.
3. Never flash an unverified full image as a first test — prefer volatile/temporary boot
   (`fastboot boot <img>`) over flashing when the platform allows it.
4. Linux sources are a **hardware reference only** — do not copy GPL kernel code into
   UEFI binaries. Read registers/addresses from them; write your own BSD-licensed code.
5. Record every magic address and hardware assumption (see `docs/01`).
6. Kernel/boot diagnostics take priority over any visual UI polish.

## Read in order

1. [`docs/01-hardware-map.md`](docs/01-hardware-map.md) — extract the `skipjack` map
2. [`docs/02-edk2-aarch32-port.md`](docs/02-edk2-aarch32-port.md) — build & bring-up
3. [`docs/03-debugging.md`](docs/03-debugging.md) — screen / GPIO blink / UART
4. [`debug/`](debug/) — reference snippets
