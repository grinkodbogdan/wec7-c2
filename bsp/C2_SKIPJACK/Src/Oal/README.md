# Src/Oal — OEM Adaptation Layer (Components B–H)

**Milestone:** M4 (core hardware). **HW map refs:** §2–§7, §12 (watchdog).

The OAL is WEC7's hardware abstraction: it initializes the platform and provides IRQ,
timer, and basic services to the kernel. Modules here are added in bring-up order.

## Modules & contracts

| Module | Component | HW map | Contract |
|--------|-----------|--------|----------|
| `debug/` | H Early Debug | §6 | **First.** Init BLSP UART (or fallback RAM/FB log); provide `OEMWriteDebugString`/byte-out. Enables all subsequent verification. |
| `memory/` | B Memory Map | §2 | Declare physical/virtual layout, reserved regions, FB carveout to the kernel. Must exclude all TrustZone/modem carveouts. |
| `intr/` | D Interrupts | §3 | GICv2 (GIC-400) init, IRQ routing, WEC7 SYSINTR mapping. |
| `timer/` | E Timers | §4 | ARMv7 arch timer as OS tick + delay; program frequency (19.2 MHz candidate). |
| `clock/` | G Clocks/Resets | §5 | GCC init; ungate/reset per-peripheral clocks on demand (no Linux CCF — explicit). |
| `gpio/` | F GPIO | §7 | TLMM pin mux/config accessors for other modules. |
| `init/` | C OAL core | §1–§7 | `OEMInit` / OAL entry that sequences the above. |

## Rules in force here

- **No magic numbers.** Every register base/offset/IRQ traces to a HW map row (Rule 8).
- **`VERIFY` values may not drive hardware.** A module referencing an unconfirmed value
  compiles only as a stub that logs "unverified" and refuses the hardware path (Rule 9).
- **Watchdog awareness:** `init/` must ensure the hardware watchdog is serviced or
  disabled early (HW map §12), or first boot will appear to randomly reset.

## Bring-up sequence (M4)

```
debug (H) → init (C) → intr (D) → timer (E) → clock (G) → gpio (F)
```
Each step must emit a debug string confirming success before the next begins (Rule 6:
smallest atomic steps).
