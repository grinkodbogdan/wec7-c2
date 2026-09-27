# Src/Bootloader — Bootloader Handoff Analysis & Shim (Component A)

**Milestone:** M2 (kernel loading) / M4. **HW map ref:** §13.1 (handoff contract).
**Risk:** Bootloader Incompatibility (HIGH), Device Bricking (MEDIUM).

## Purpose

The C2 ships with an **unlocked** Qualcomm `aboot`-style bootloader that natively loads
Android `boot.img`. WEC7 needs it to load and jump to an `NK.bin`. This folder holds the
analysis of that gap and any shim needed to bridge it.

## Open contract (all VERIFY)

- Does `aboot` accept `fastboot boot <image>` of a raw/`NK.bin` payload, or only a
  signed/formatted Android `boot.img`?
- What is the load address and header format it expects?
- What CPU/register/MMU state does it hand off with (feeds `Src/Startup`)?
- Is a small "Android `boot.img` wrapper around `NK.bin`" the least-risky first test?

## Method (safety-gated)

1. **Analysis only first** — study AsteroidOS boot flow, `aboot` behavior, boot image
   format. No flashing.
2. **Volatile boot only** — first on-device test is `fastboot boot` of a *verified* tiny
   image (e.g. one that just spins/toggles a GPIO or prints to UART), never
   `fastboot flash` (Rule 4).
3. M0 backups must be complete and verified first (Rule 2).

No code here executes on-device until the M0 gate is cleared and §13.1 is understood.
