# Hardware Reference Sources & Usage Policy

Mirrors spec §5. **All four sources are references for hardware facts only.** Per Rule
5, no Linux binary or driver is reused as WEC7 code — these inform *what the hardware
is*, not *what to link*.

| Source | Provides | Usage policy |
|--------|----------|--------------|
| **AsteroidOS C2 / skipjack** | Device tree source (`.dts`/`.dtb`), kernel source, platform drivers, boot parameters, board descriptions. | Hardware & register reference **only**. Linux code is not reusable as WEC7 binaries. Extract addresses, IRQs, clock names, pin muxes, panel init sequences (as *reference* for re-implementation). |
| **Void Linux environment** | Live runtime diagnostics: `dmesg`, `/proc`, `/sys`, active module configs. | Verify hardware addresses and power-on state behavior on the *actual device*. This is how `FAMILY` values become `CONFIRMED`. |
| **Qualcomm MSM8909/W docs** | Register definitions, clock specs, pinout muxing, peripheral controller docs. | Specification reference for the custom OAL implementation. |
| **WEC7 reference BSPs** | Standard BSP folder structure, OAL scaffolding, ARM startup conventions, driver interfaces. | Starting-point scaffolding for the C2 BSP. Does **not** natively support MSM8909W — adapt, don't assume. |

## Extraction workflow

1. Pull the AsteroidOS `skipjack` device tree and locate each peripheral node.
2. Cross-check every address/IRQ against the live Void Linux `/proc/iomem`,
   `/proc/interrupts`, `/sys/kernel/debug/*` on the physical device.
3. Cross-reference against Qualcomm MSM8909 documentation for register semantics.
4. Record the confirmed value + its source in `docs/01-hardware-map.md`, promoting the
   confidence level.

## Attaching the sources to this repo

The AsteroidOS tree and Void runtime dumps are **not** stored in this repo (kernel trees
are large; runtime dumps are device-specific). When captured:

- Store raw device dumps under `docs/references/hw-dumps/<date>/` (create as needed) and
  reference specific files from the HW map's Source column.
- Link (don't vendor) the AsteroidOS `skipjack` tree; cite specific dts nodes by path.

> **Session note:** as of the initial scaffold, neither the AsteroidOS `skipjack` tree
> nor a live Void Linux dump is attached to the working session. This is why the HW map
> currently tops out at `FAMILY` confidence. Attaching them is the prerequisite to
> completing M1.
