# Roadmap & Milestone Tracker (M0 – M10)

Living tracker for the WEC7-C2 port. Update the **Status** and **Notes** as work
proceeds. Statuses: `NOT STARTED` · `IN PROGRESS` · `BLOCKED` · `DONE`.

## Success criteria (M1–M6, per spec §3)

| # | Milestone | Status | Notes |
|---|-----------|--------|-------|
| M1 | BSP Buildability | NOT STARTED | Needs M2 toolchain. |
| M2 | Kernel Loading | NOT STARTED | Blocked on bootloader handoff contract (HW map §13.1). |
| M3 | Kernel Execution | NOT STARTED | Needs debug channel (HW map §6). |
| M4 | Core Hardware | NOT STARTED | RAM/IRQ/timers/GPIO/clocks. |
| M5 | Input & Storage | NOT STARTED | |
| M6 | Power Management | NOT STARTED | |

> Note: the spec uses "M1–M6" for the *success criteria* (§3) and "M0–M10" for the
> *development phases* (§6). They are related but numbered independently. The phase
> tracker below is the primary work breakdown.

## Development phases (M0 – M10, per spec §6)

| Phase | Title | Status | Deliverable / Exit criteria | Doc |
|-------|-------|--------|------------------------------|-----|
| **M0** | Recovery Plan | IN PROGRESS | Verified partition backups + documented restore. | `02-recovery-plan.md` |
| **M1** | Hardware Mapping | IN PROGRESS | Full register map at CONFIRMED confidence. | `01-hardware-map.md` |
| **M2** | Development Toolchain | NOT STARTED | Isolated, reproducible PB env builds stock BSP. | `03-toolchain-setup.md` |
| **M3** | C2 BSP Skeleton | IN PROGRESS (scaffold) | Workspace, OAL config, linker scripts, startup asm. | `bsp/` |
| **M4** | First Kernel Boot | BLOCKED | Deterministic kernel exec + serial debug on HW. | HW map §6, §13 |
| **M5** | Display bring-up | NOT STARTED | 360×360 framebuffer updates. | HW map §8 |
| **M6** | Touch bring-up | NOT STARTED | Touch events with correct coordinates. | HW map §9 |
| **M7** | eMMC / storage | NOT STARTED | Persistent filesystem. | HW map §10 |
| **M8** | PMIC / power | NOT STARTED | Battery/charger/watchdog/suspend. | HW map §12 |
| **M9** | Wireless | NOT STARTED | BT/Wi-Fi (later phase). | — |
| **M10** | Watch shell | NOT STARTED | Custom UI shell. | — |

## Current focus

1. **M0** — enumerate partitions and capture verified backups (safety gate).
2. **M1** — promote hardware-map values from `FAMILY`/`VERIFY` to `CONFIRMED` by
   extracting from the AsteroidOS `skipjack` tree and the live Void Linux instance.
3. **M4 unblocking research** — the four open questions in HW map §13 (bootloader
   handoff, debug channel, reserved-memory carveouts, watchdog behavior).

## Blockers

| Blocker | Blocks | Owner action |
|---------|--------|--------------|
| AsteroidOS `skipjack` dts not attached to session | M1 CONFIRMED values | Attach kernel tree / dts. |
| Live Void Linux runtime dumps unavailable | M1 CONFIRMED values | Capture `/proc`,`/sys`,`dmesg` per HW map §0. |
| Bootloader handoff contract unknown | M2, M4 | Boot-protocol analysis (no flashing). |
| Legacy WEC7 Platform Builder not provisioned | M1–M4 | Stand up isolated VM (M2). |
