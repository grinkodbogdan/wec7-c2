# WEC7-C2 Board Support Package

This tree holds the C2 (`skipjack`) WEC7 BSP. It is currently a **skeleton (M3)**: a
folder layout and per-component contracts, with **no hardware-touching code yet**. Code
is added only against `CONFIRMED` rows in `docs/01-hardware-map.md` (Rule 8, Rule 9).

## BSP component architecture (spec §4)

The BSP bridges WEC7 to the Snapdragon Wear 2100 (MSM8909W) via core init + OAL modules:

| ID | Component | Skeleton location | Milestone | HW map ref |
|----|-----------|-------------------|-----------|------------|
| A | Boot & Startup — ARM startup, CPU init, kernel entry handoff, boot args, memory params. | `Src/Startup/`, `Src/Bootloader/` | M3/M4 | §1, §2, §13 |
| B | Memory Map — 512 MB layout, reserved regions, FB/driver reservations. | `Src/Oal/` (memory config) | M3/M4 | §2 |
| C | OAL Layer — system init, platform IRQ routing, timers, basic services. | `Src/Oal/` | M4 | §3–§5 |
| D | Interrupts — GIC-400 identification, routing, WEC7 mapping. | `Src/Oal/` (interrupt module) | M4 | §3 |
| E | System Timers — hardware timer id, frequency, scheduler tick. | `Src/Oal/` (timer module) | M4 | §4 |
| F | GPIO Controller — TLMM pin mapping for display/input/power/buses. | `Src/Oal/` (gpio module) | M4 | §7 |
| G | Clocks & Resets — GCC init, domain gating, reset routines. | `Src/Oal/` (clock module) | M4 | §5 |
| H | Early Debug — UART / low-level log before drivers. | `Src/Oal/` (debug module) | **M4 (first)** | §6 |
| I | Display Driver — controller, panel, framebuffer, 360×360. | `Src/Drivers/Display/` | M5 | §8 |
| J | Touch Input — I2C/SPI panel, touch IRQ, coord scaling. | `Src/Drivers/Touch/` | M6 | §9 |
| K | Storage Stack — eMMC block drivers, partitions, FS. | `Src/Drivers/Storage/` | M7 | §10, §11 |
| L | Power / Battery — PMIC reporting, charger, watchdog, suspend/resume. | `Src/Drivers/Power/` | M8 | §12 |

## Bring-up order (Rule 10: diagnostics before UI)

```
H Early Debug ─► C OAL init ─► D Interrupts ─► E Timers ─► G Clocks ─► F GPIO
   (M4 first)                        └──────── M4 core hardware ────────┘
                                                   │
                          I Display (M5) ─► J Touch (M6) ─► K Storage (M7) ─► L Power (M8)
```

**Early Debug (H) is implemented first** — without it, first boot is blind and M4 cannot
be verified.

## Standard WEC7 BSP mapping

This skeleton follows WEC7 `PLATFORM\<BSP>` conventions loosely:

| WEC7 concept | Here |
|--------------|------|
| `Src\Bootloader` | `Src/Bootloader/` — bootloader/handoff shim research. |
| `Src\Oal` | `Src/Oal/` — the OEM Adaptation Layer (components B–H). |
| `Src\Kernel\Oal` startup | `Src/Startup/` — ARM startup assembly, CPU init. |
| `Src\Drivers` | `Src/Drivers/` — peripheral drivers (I–L). |
| `Src\Inc` | `Src/Inc/` — shared headers (register defs from HW map). |
| `Files` | `Files/` — image config (`platform.bib`, `platform.reg`, etc.) — TBD. |

> When the real Platform Builder workspace is provisioned (M2), these folders map into an
> actual PB `PLATFORM` directory; the layout here is source-controllable and
> host-agnostic so it can be diffed and reviewed without the legacy toolchain.

## Status

**Skeleton only.** No `.c`/`.s` implementation is committed yet — every component has a
`README.md` contract stating its inputs (HW map refs), outputs, and milestone gate.
Implementation begins after M2 (toolchain) and the relevant HW map rows are `CONFIRMED`.
