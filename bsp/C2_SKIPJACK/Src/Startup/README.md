# Src/Startup — ARM Startup & CPU Init (Component A)

**Milestone:** M3 (skeleton) → M4 (first execution). **HW map refs:** §1 (CPU), §2
(memory), §13 (handoff contract).

## Contract

- **Entry:** the first instruction the WEC7 kernel image runs after the bootloader hands
  off (`NK.bin` entry). Environment at entry (register state, MMU on/off, cache state)
  is defined by the bootloader handoff contract — **currently `VERIFY`** (HW map §13.1).
- **Responsibilities:**
  1. Establish a known CPU state (ARMv7-A, supervisor mode, interrupts masked).
  2. Set up initial stack(s).
  3. Configure MMU / initial page tables per the WEC7 memory map (HW map §2).
  4. Enable caches, VFP/NEON (HW map §1).
  5. **Service/disable the hardware watchdog if the bootloader armed it** (HW map §12).
  6. Bring up the Early Debug channel ASAP (component H) and emit a deterministic
     "kernel entry" string — this is the M4 proof.
  7. Jump to the OAL/kernel init.
- **Output:** control passed to OAL with a valid execution environment + working debug
  output.

## Blocking unknowns (must be CONFIRMED before real code)

- Bootloader handoff register/MMU/cache state (HW map §13.1).
- Kernel load/link address vs. reserved-memory carveouts (HW map §2, §13.3).
- Watchdog armed at handoff? timeout? (HW map §12, §13.4).

No startup assembly is committed until these are resolved (Rule 8/9).
