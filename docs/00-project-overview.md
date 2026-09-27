# Project Overview — WEC7-C2

## Objective

Port **Windows Embedded Compact 7 (WEC7)** to the **Mobvoi TicWatch C2** (codename
`skipjack`) as a native ARM32 / ARMv7-A embedded OS. First success = a real WEC7 kernel
executing on physical hardware with serial/debug output. UI and connectivity are later
phases.

## Why WEC7

WEC7 is a lightweight, hard real-time embedded OS built for constrained ARM hardware
with first-class support for custom **Board Support Packages (BSP)** and an **OEM
Adaptation Layer (OAL)**. That BSP/OAL model is what makes porting to an undocumented
SoC tractable at all.

## Explicitly out of scope (do not drift into these)

- Windows 10 ARM64 / ARM32 Desktop
- Windows PE environments
- QEMU / software emulation targets

The target is **real WEC7 on real C2 hardware**. Emulation is not a substitute
milestone.

## Target at a glance

| | |
|---|---|
| Device | Mobvoi TicWatch C2 (`skipjack`) |
| SoC | Qualcomm Snapdragon Wear 2100 (MSM8909W) |
| CPU | Quad Cortex-A7, ARMv7-A, 32-bit |
| RAM | 512 MB |
| Display | ~360×360 round panel |
| Bootloader | Unlocked |

## Strategy

1. **Discover, then implement** (Rule 9). The hardware map (`docs/01-hardware-map.md`)
   is produced *first* and treated as the binding spec. Drivers are written against
   confirmed facts, not guesses.
2. **Safety first** (Rules 1–4). Full partition backups (M0) and volatile boot precede
   any flashing.
3. **Boot before beauty** (Rule 10). Serial/debug output and core hardware (M4) rank
   above display and UI.
4. **Reference, don't reuse** (Rule 5). AsteroidOS/Void Linux/QC docs supply hardware
   facts; no Linux binary is reused as WEC7 code.
5. **Smallest atomic steps** (Rule 6) with **reproducible** build scripts (Rule 7).

## Milestone dependency chain

```
M0 Recovery ──► M1 HW Map ──► M2 Toolchain ──► M3 BSP Skeleton ──► M4 First Boot
                                                                        │
        ┌───────────────┬───────────────┬──────────┬──────────┬────────┘
        ▼               ▼               ▼          ▼          ▼
   M5 Display      M6 Touch       M7 eMMC     M8 PMIC    M9 Wireless ──► M10 Shell
```

M0 and M1 can proceed in parallel (both are non-destructive discovery/backup). M2 can
overlap. Nothing on-device past volatile boot happens before M0 is complete and the M4
boot contract (`docs/01-hardware-map.md` §13) is understood.

## Glossary

| Term | Meaning |
|------|---------|
| **BSP** | Board Support Package — the SoC/board-specific software layer. |
| **OAL** | OEM Adaptation Layer — WEC7's HAL: init, interrupts, timers, basic services. |
| **NK.bin** | The WEC7 kernel image the bootloader loads. |
| **Platform Builder** | Legacy Microsoft toolchain that builds WEC7 OS images. |
| **skipjack** | Mobvoi's internal codename for the TicWatch C2. |
| **MSM8909W** | The Snapdragon Wear 2100 SoC part number ("W" = wearable). |
| **TLMM** | Qualcomm Top-Level Mode Multiplexer (the GPIO/pinmux block). |
| **BLSP** | Qualcomm BAM Low-Speed Peripheral (hosts UART/I2C/SPI QUP engines). |
| **SPMI** | System Power Management Interface (bus to the PMIC). |
| **EDL / 9008** | Qualcomm Emergency Download mode — last-resort recovery. |

See also: `05-risk-register.md`, `06-engineering-rules.md`, `references/reference-sources.md`.
