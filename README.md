# WEC7-C2 — Windows Embedded Compact 7 Port for the Mobvoi TicWatch C2

> **Status:** Exploratory / BSP Design
> **Target:** Mobvoi TicWatch C2 (codename `skipjack`) — Qualcomm Snapdragon Wear 2100 (MSM8909W), ARM32 / ARMv7-A
> **Primary objective:** Execute a real WEC7 ARM kernel on physical C2 hardware with serial/debug diagnostics.

This repository hosts the engineering effort to port **Windows Embedded Compact 7
(WEC7)** to the Mobvoi TicWatch C2 as a native ARM32 embedded OS. A full smartwatch
UI and advanced connectivity are explicitly *later-phase* milestones. The first
success criterion is a booting kernel with debug output — nothing else.

The authoritative source of scope and requirements is
[`TicWatch C2 WEC7 Project Specifications.pdf`](./TicWatch%20C2%20WEC7%20Project%20Specifications.pdf).
This README and the `docs/` tree operationalize that specification; where they ever
disagree, the PDF wins until the discrepancy is reconciled here.

---

## Sequential success criteria

| # | Milestone | Definition of done |
|---|-----------|--------------------|
| M1 | BSP Buildability | A C2-specific WEC7 BSP compiles under the Platform Builder toolchain. |
| M2 | Kernel Loading | The C2 bootloader loads the WEC7 ARM kernel image (`NK.bin`) into RAM. |
| M3 | Kernel Execution | Kernel executes far enough to emit serial / low-level debug output. |
| M4 | Core Hardware | RAM, interrupts, timers, GPIO, and clocks are brought up. |
| M5 | Input & Storage | Touchscreen, buttons, and persistent storage function. |
| M6 | Power Management | Battery, charger, watchdog, and suspend/resume function. |

## Development roadmap (M0 – M10)

See [`docs/04-roadmap.md`](./docs/04-roadmap.md) for the living tracker.

| Phase | Title | Summary |
|-------|-------|---------|
| **M0** | Recovery Plan | Back up original partitions; document fastboot restore. **← safety gate for everything.** |
| **M1** | Hardware Mapping | Compile the full register map (CPU, RAM, IRQ, timers, GPIO, display, touch). |
| **M2** | Development Toolchain | Stand up a legacy WEC7 Platform Builder environment, isolated from the Debian host. |
| **M3** | C2 BSP Skeleton | Workspace structure, base OAL config, linker scripts, startup assembly. |
| **M4** | First Kernel Boot | Deterministic WEC7 kernel execution + serial debug on hardware. |
| **M5–M10** | Peripherals & Shell | Display (M5), touch (M6), eMMC (M7), PMIC (M8), wireless (M9), watch shell (M10). |

---

## Repository layout

```
.
├── README.md                          This file.
├── TicWatch C2 WEC7 Project Specifications.pdf   Binding specification.
├── docs/
│   ├── 00-project-overview.md         Architecture, scope, glossary.
│   ├── 01-hardware-map.md             ★ Immediate deliverable: the binding HW spec.
│   ├── 02-recovery-plan.md            M0 — partition backup & restore procedures.
│   ├── 03-toolchain-setup.md          M2 — WEC7 Platform Builder environment.
│   ├── 04-roadmap.md                  M0–M10 milestone tracker.
│   ├── 05-risk-register.md            Risks & mitigations.
│   ├── 06-engineering-rules.md        The 10 non-negotiable rules.
│   └── references/
│       └── reference-sources.md       AsteroidOS / Void Linux / QC docs usage policy.
└── bsp/
    ├── README.md                      BSP component architecture (A–L).
    └── C2_SKIPJACK/                    WEC7 BSP folder skeleton (M3).
```

---

## Non-negotiable engineering rules (do not skip)

1. **Preserve** existing working AsteroidOS and Void Linux installations.
2. **Always** perform raw partition backups before testing boot changes.
3. Unlocked bootloaders **do not** guarantee brick recovery — exercise caution.
4. **Never** flash an unverified full image build as an initial test.
5. Use Linux sources purely as **hardware references**, not as binary code.
6. Test the **smallest atomic** boot-stage change possible.
7. Keep all BSP build scripts and environment rules **reproducible**.
8. **Explicitly record** every hardware assumption and magic register address.
9. **Decouple** hardware discovery from driver implementation.
10. **Kernel boot diagnostics take priority** over visual UI work.

Full text and rationale: [`docs/06-engineering-rules.md`](./docs/06-engineering-rules.md).

---

## Where the project stands right now

- **Proven:** unlocked bootloader present; hardware runs ARM Linux (AsteroidOS / Void
  Linux); 512 MB RAM validated; target confirmed as ARM32 WEC7.
- **Unproven:** WEC7 toolchain readiness; whether the C2 bootloader can parse and hand
  control to a WEC7 `NK.bin`; initial kernel execution feasibility.
- **Immediate next task:** finish the **hardware map** (`docs/01-hardware-map.md`) by
  extracting confirmed values from the AsteroidOS `skipjack` device tree and the live
  Void Linux instance. Fields still marked `VERIFY` are blocked on access to those
  sources.
