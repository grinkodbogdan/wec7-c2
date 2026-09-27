# Project Risk Register

Mirrors spec §7. Update mitigations and status as the project learns more.

| # | Risk | Severity | Mitigation | Status |
|---|------|----------|------------|--------|
| R1 | **Legacy WEC7 tooling** — build env is old, Windows-only, fragile. | HIGH | Isolate legacy Windows build environments inside dedicated virtual instances, outside the main Debian host. Snapshot + version-manifest for reproducibility (Rule 7). | Open — see `03-toolchain-setup.md`. |
| R2 | **No MSM8909W WEC7 BSP** exists. | HIGH | Adapt a standard ARMv7 reference BSP, using AsteroidOS / Void Linux hardware specs as the register guide (reference only, Rule 5). | Open — HW map is the register guide. |
| R3 | **Bootloader incompatibility** — C2 `aboot` may not parse/hand off to `NK.bin`. | HIGH | Deep boot-protocol analysis **before** any flash. Use volatile `fastboot boot` first. | Open — HW map §13.1. |
| R4 | **Missing drivers** — most peripherals unsupported initially. | HIGH | Implement minimal driver **stubs** aligned to the current milestone only (OAL before graphics, Rule 9/10). | Open — BSP skeleton stubs. |
| R5 | **Lack of early debug UART** — first boot may be blind. | MEDIUM | Investigate HW test points; fall back to RAM log buffer or framebuffer logging. | Open — HW map §6. |
| R6 | **Device bricking / software damage.** | MEDIUM | Maintain full raw partition dumps; verify fastboot/EDL restore paths **before** flashing (Rules 2, 3). | Mitigated by M0 gate — `02-recovery-plan.md`. |

## Escalation notes

- Any HIGH risk that turns into a confirmed blocker gets an entry in
  `04-roadmap.md#blockers`.
- R3 and R6 together define the hard rule: **no `fastboot flash` until M0 backups are
  verified and the handoff contract is understood.** Volatile boot only.
