# Non-Negotiable Engineering Rules

These ten rules (spec §8) govern every decision on this project. They are not
guidelines; a change that violates one of them is wrong regardless of how convenient it
is. Each is annotated with how it is enforced here.

| # | Rule | How we enforce it |
|---|------|-------------------|
| 1 | **Preserve** existing working AsteroidOS and Void Linux installations. | AsteroidOS + Void boot is the M0 baseline check and a required exit criterion after every on-device experiment. |
| 2 | **Always** perform raw partition backups before testing boot changes. | M0 gate (`02-recovery-plan.md`). No boot-affecting step proceeds without verified backups. |
| 3 | Unlocked bootloader **does not** guarantee brick recovery — exercise caution. | Volatile-boot-first strategy; EDL/fastboot restore paths confirmed before flashing. |
| 4 | **Never** flash an unverified full image as an initial test. | First on-device step is `fastboot boot` of a small, verified test image — never `fastboot flash` of a full build. |
| 5 | Use Linux sources purely as **hardware references**, not binary code. | AsteroidOS/Void code informs the HW map and driver logic only; no Linux binary is linked or reused. Every HW map row cites a *fact*, not code. |
| 6 | Test the **smallest atomic** boot-stage change possible. | Milestones are decomposed to the smallest observable step (e.g. "emit one string from kernel entry" is its own gate). |
| 7 | Keep all BSP build scripts and environment rules **reproducible**. | M2 requires a version manifest + ordered install script that rebuilds the toolchain from scratch (`03-toolchain-setup.md`). |
| 8 | **Explicitly record** every hardware assumption and magic register address. | The HW map's confidence column (`CONFIRMED`/`FAMILY`/`VERIFY`) + Source column. No unsourced magic numbers in code. |
| 9 | **Decouple** hardware discovery from driver implementation. | Discovery lives in `01-hardware-map.md`; drivers are written only against `CONFIRMED` rows. |
| 10 | **Kernel boot diagnostics take priority** over visual UI work. | Roadmap orders M4 (boot+serial) strictly before M5 (display) and M10 (shell). |

## Practical corollaries

- **No unsourced magic numbers in code.** Every register address, IRQ, or offset in the
  BSP must trace to a row in `docs/01-hardware-map.md`. If it's not in the map, it
  doesn't go in the code.
- **`VERIFY` blocks merge into drivers.** A driver may reference a `VERIFY` value only in
  a stub that refuses to run / logs "unverified," never in a code path that touches
  hardware.
- **Backups are never committed.** Device firmware dumps stay out of git (see
  `.gitignore`).
- **Reproducible or it didn't happen.** A build that can't be reproduced from the
  manifest is treated as broken.
