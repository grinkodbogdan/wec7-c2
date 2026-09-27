# C2 / skipjack Hardware Map — Binding Specification for the WEC7 BSP/OAL

> **This document is the binding hardware specification for the WEC7 BSP/OAL** (per
> spec §9). Every address, IRQ, clock, and pin the OAL and drivers rely on must be
> recorded here first, with its source and confidence.
>
> **Confidence legend**
> - `CONFIRMED` — verified against the actual skipjack device tree *and/or* the live
>   Void Linux `/proc`,`/sys` on this exact device.
> - `FAMILY` — documented for the MSM8909 / Snapdragon Wear 2100 family in public
>   Qualcomm / mainline sources; highly likely but **not yet** confirmed on skipjack.
> - `VERIFY` — placeholder / unknown; **must** be extracted before any driver depends
>   on it. Blocks the milestone that needs it.
>
> **Provenance note.** The AsteroidOS `skipjack` kernel tree and the live Void Linux
> runtime are the required extraction sources (spec §5). They are **not attached to
> the current work session**, so no value below is marked `CONFIRMED` yet. Values are
> seeded at `FAMILY` confidence from public MSM8909 documentation to give drivers a
> starting point, and downgraded to `VERIFY` where even the family value is uncertain
> for a wearable variant. Per Rule 5, none of this implies reuse of Linux binaries —
> only hardware facts. Per Rule 8, every magic address is recorded with its source.

---

## 0. Extraction procedure (how to promote FAMILY → CONFIRMED)

Run these against the device to confirm each section. Record raw output under
`docs/references/` (create `hw-dumps/` when captured) and cite it in the "Source"
column.

| Datum | AsteroidOS source | Void Linux runtime command |
|-------|-------------------|----------------------------|
| DDR base / size | `memory@…` node in the `skipjack` `.dts`/`.dtb` | `cat /proc/iomem`, `free -m`, `cat /proc/meminfo` |
| SoC / CPU | `compatible`, `cpus` nodes | `cat /proc/cpuinfo`, `cat /proc/device-tree/compatible` |
| Interrupt controller | `interrupt-controller@…` node | `cat /proc/interrupts`, `/sys/.../interrupt-controller` |
| Timers | `timer@…`, `arch_timer` nodes | `dmesg | grep -i -E 'timer|clocksource|arch_timer'` |
| Clocks (GCC) | `clock-controller@…` | `cat /sys/kernel/debug/clk/clk_summary` (if debugfs) |
| GPIO / pinmux | `tlmm`/`pinctrl@…`, `*-pins` | `cat /sys/kernel/debug/gpio`, `/sys/kernel/debug/pinctrl/*` |
| UART (debug) | `serial@…`, chosen `stdout-path` | `dmesg | grep -i -E 'ttyMSM|ttyHSL|uart|console'` |
| Display / DSI | `mdss`,`mdp`,`dsi`,`panel@…` | `dmesg | grep -i -E 'mdss|dsi|panel|drm'`, `/sys/class/drm/*` |
| Touch | `i2c`/`spi` touch node | `cat /proc/bus/input/devices`, `dmesg | grep -i touch`, `i2cdetect` |
| Storage (eMMC) | `sdhci@…`/`mmc@…` | `cat /proc/partitions`, `lsblk`, `dmesg | grep -i -E 'mmc|sdhci'` |
| PMIC | `spmi@…`, `pm8909` nodes | `dmesg | grep -i -E 'spmi|pm8909|pmic|regulator'` |
| Partitions | boot args / `mtd`/GPT | `cat /proc/partitions`, `ls -l /dev/block/bootdevice/by-name/` |

---

## 1. SoC & CPU

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| SoC | Qualcomm **MSM8909W** (Snapdragon Wear 2100) | FAMILY | Spec §1. "W" = wearable bin. |
| CPU cluster | 4× ARM **Cortex-A7**, ARMv7-A | FAMILY | Spec §1. Quad-core. |
| Max clock | ~1.1–1.2 GHz | VERIFY | Wearable bins are often down-clocked; confirm from cpufreq. |
| FPU / SIMD | VFPv4 + NEON | FAMILY | Standard Cortex-A7. Enable in OAL CPU init. |
| Cache | L1 32K I/D per core; L2 shared (256–512K) | VERIFY | Confirm L2 size from CP15 / cpuinfo. |
| GPU | Adreno 304 | FAMILY | Not needed until M5; WEC7 uses SW/GDI framebuffer first. |
| DDR base address | `0x80000000` | FAMILY | Qualcomm MSM DDR base. **Anchors the WEC7 memory map.** |
| DDR size | 512 MB (`0x20000000`) | FAMILY | Spec §1. Confirm usable size after carveouts. |

**OAL action:** set `ARMv7-A`, enable VFP/NEON, MMU on, kernel virtual base per WEC7
convention (`0x80000000` physical → WEC7 static-mapped region).

---

## 2. Memory map (physical)

> WEC7 needs: kernel RAM, a framebuffer carveout, and any firmware-reserved regions
> (modem/SMEM/tz) it must **not** touch. On Qualcomm parts several low regions are
> reserved by TrustZone/SMEM — clobbering them hangs the boot.

| Region | Phys base | Size | Confidence | Note |
|--------|-----------|------|------------|------|
| DDR (start) | `0x80000000` | 512 MB | FAMILY | Top of usable RAM = base + size − carveouts. |
| SMEM (shared memory) | `0x8FF00000`-ish | ~2 MB | VERIFY | Reserved by firmware; extract from `/proc/iomem` + dts `reserved-memory`. |
| TrustZone / modem carveouts | — | — | VERIFY | **Critical:** enumerate all `reserved-memory` nodes before choosing kernel/FB regions. |
| WEC7 kernel load region | TBD | TBD | VERIFY | Choose after carveouts known; align to bootloader load address (§9). |
| Framebuffer carveout | TBD | 360×360×4 ≈ 512 KB (+padding) | VERIFY | Reserve contiguous; align to panel driver requirements (M5). |

**Rule 9:** this table is *discovery*. Do not implement the FB allocator until the
carveout map is `CONFIRMED`.

---

## 3. Interrupt controller

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Type | ARM **GIC-400** (GICv2) | FAMILY | MSM8909 family uses GICv2. |
| Distributor (GICD) base | `0x0B000000` | FAMILY | MSM8909/8916-family typical. **VERIFY vs skipjack dts.** |
| CPU interface (GICC) base | `0x0B002000` | FAMILY | Same family map. |
| SPI base / count | 32+ SPIs | VERIFY | Per-peripheral IRQ numbers extracted per driver, below. |

**OAL action:** implement GICv2 init in the interrupt module (BSP component **D**).
Per-device IRQ line numbers get filled in as each driver is scoped (Rule 9: routing
first, handlers later).

---

## 4. System timers

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Primary | ARMv7 **architected timer** (CP15 generic timer) | FAMILY | Preferred OS tick + delay source. |
| Arch-timer frequency | 19.2 MHz (`19200000`) | FAMILY | Qualcomm XO-derived; confirm `CNTFRQ`. |
| Secondary | Qualcomm **MSM/QTIMER** memory-mapped | FAMILY | Fallback / SP804-style. Base `VERIFY`. |
| OS scheduler tick | derive from arch timer | — | BSP component **E**. |

---

## 5. Clocks & resets

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Global Clock Controller (GCC) base | `0x01800000` | FAMILY | MSM8909 GCC. **VERIFY.** |
| Crystal / XO | 19.2 MHz | FAMILY | Root of clock tree. |
| Reset controller | within GCC (BCR blocks) | FAMILY | BSP component **G**: gate/ungate per peripheral before use. |

**Note:** WEC7 has no Linux common-clock framework. The OAL must ungate each
peripheral clock explicitly before touching that peripheral's registers. Extract the
needed `GCC_*_CBCR` offsets per driver from MSM8909 clock docs.

---

## 6. Early debug UART (highest priority — enables M4)

> Rule 10 + risk "Lack of Early Debug UART" (MEDIUM). Without serial out, first-boot
> is blind. This is the single most valuable datum to `CONFIRM` early.

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Controller type | Qualcomm **BLSP QUP UART** (`msm_serial`, DM mode) | FAMILY | MSM8909 uses BLSP UARTs. |
| Debug UART base | `0x078B0000` (BLSP1 UART2) *candidate* | VERIFY | msm8916-family address; confirm which BLSP the C2 console uses via `stdout-path`. |
| Alt candidate | `0x078AF000` (BLSP1 UART1) | VERIFY | Confirm from AsteroidOS chosen node / dmesg console. |
| Baud | 115200 8N1 (assumed) | VERIFY | Confirm from bootargs. |
| Physical test point | unknown | VERIFY | Investigate PCB test points; a wearable may not expose UART on a header. |

**Fallback debug channels** (per risk register, if no UART pinout is exposed):
1. **RAM log buffer** — write early prints to a fixed DDR address; dump post-mortem
   from the bootloader or a Linux boot.
2. **Framebuffer logging** — once the display carveout is known, paint text directly
   (later, higher effort).

**M4 gate:** first kernel boot is not "achieved" until *some* debug channel emits a
deterministic string from kernel entry.

---

## 7. GPIO / pin mux

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Controller | Qualcomm **TLMM** | FAMILY | MSM8909 top-level mux. |
| TLMM base | `0x01000000` | FAMILY | Family typical. **VERIFY.** |
| GPIO count | ~117 | FAMILY | MSM8909. Confirm from `/sys/kernel/debug/gpio`. |
| Buttons (side key) | pin(s) TBD | VERIFY | Extract from dts `gpio-keys`. Needed M5. |
| Display reset / TE | pins TBD | VERIFY | Needed M5. |
| Touch INT / reset | pins TBD | VERIFY | Needed M6. |

BSP component **F**. Each consumed pin gets one row here before its driver is written.

---

## 8. Display

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Panel | Round OLED, **~360×360** | FAMILY | Spec §1. Confirm exact res & timing. |
| Interface | MIPI **DSI** via Qualcomm MDSS/MDP | FAMILY | MSM8909 display subsystem. |
| MDSS/MDP base | TBD | VERIFY | Extract from dts `mdss`/`mdp` nodes. |
| Panel controller / init sequence | unknown | VERIFY | Panel-specific DSI command set — hardest single item; extract from AsteroidOS panel driver as *reference* (Rule 5). |
| Pixel format | assume RGB888 / 32bpp FB | VERIFY | Confirm; affects FB carveout size (§2). |

Deferred to **M5**. Do not start before M4 boot proven (Rule 10).

---

## 9. Touch input

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Bus | I2C (BLSP QUP) — assumed | VERIFY | Could be I2C or SPI; confirm. |
| Controller IC | unknown | VERIFY | Extract from dts touch node compatible + `i2cdetect`. |
| I2C address | TBD | VERIFY | From `i2cdetect` on the touch bus. |
| INT / reset GPIO | TBD | VERIFY | See §7. |
| Coordinate space | 360×360, scaling TBD | VERIFY | Map raw → panel coords in driver (BSP component **J**). |

Deferred to **M6**.

---

## 10. Storage (eMMC)

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| Controller | Qualcomm **SDHCI (SDCC)** | FAMILY | MSM8909 eMMC. |
| eMMC controller base | `0x07824000` (sdhc_1 candidate) | VERIFY | msm8916-family; confirm on skipjack. |
| Partition scheme | GPT (`by-name`) | FAMILY | Extract full table (§11). |

Deferred to **M7**.

---

## 11. Partition table (fill from device — feeds M0 recovery)

> Extract with `ls -l /dev/block/bootdevice/by-name/` and `cat /proc/partitions` on
> Void Linux. **This table is the input to the M0 backup plan** — every partition here
> must have a backup before any flash test (Rule 2). See `docs/02-recovery-plan.md`.

| Name | Block device | Start | Size | Purpose | Backed up? |
|------|--------------|-------|------|---------|------------|
| `sbl1` / `xbl` | VERIFY | — | — | Primary bootloader | ☐ |
| `aboot` / `bootloader` | VERIFY | — | — | Applications bootloader (fastboot) | ☐ |
| `boot` | VERIFY | — | — | Kernel + ramdisk | ☐ |
| `recovery` | VERIFY | — | — | Recovery image | ☐ |
| `system` | VERIFY | — | — | Root FS | ☐ |
| `modem` / `mdtp` | VERIFY | — | — | Firmware — **do not touch** | ☐ |
| `tz` | VERIFY | — | — | TrustZone — **do not touch** | ☐ |
| … (complete from device) | | | | | ☐ |

---

## 12. Power / battery / PMIC

| Property | Value | Confidence | Source / Note |
|----------|-------|------------|---------------|
| PMIC | Qualcomm **PM8909** | FAMILY | Companion to MSM8909. |
| Access bus | **SPMI** | FAMILY | Extract SPMI base + PM8909 sid. |
| SPMI base | TBD | VERIFY | From dts `spmi@…`. |
| Battery fuel gauge | PM8909 VADC/BMS | VERIFY | Reporting path for M8. |
| Charger IC | unknown | VERIFY | Could be PMIC-integrated (linear) or external. |
| Hardware watchdog | MSM watchdog (APCS) | FAMILY | **Must be serviced or disabled early** or the SoC resets mid-boot — relevant to M4. |
| Suspend / resume | TBD | VERIFY | M6 / M8. |

**M4 watchdog caveat:** confirm whether the bootloader arms a hardware watchdog before
handoff. If so, the OAL must pet or disable it immediately on entry, or first boot will
appear to "reset randomly." Record the watchdog base/timeout here once known.

---

## 13. Open questions blocking progress

1. **Bootloader handoff contract (blocks M2/M4).** Can C2 `aboot` parse and jump to a
   WEC7 `NK.bin`, or only Android `boot.img`? What load address, header format, and
   register/`r0-r2` state does it hand off with? → deep boot-protocol analysis, no
   flashing (risk: Bootloader Incompatibility, HIGH).
2. **Debug channel (blocks M4).** Is any UART exposed on a test point? (§6)
3. **Reserved-memory map (blocks M2/M4).** Full TrustZone/modem carveout list. (§2)
4. **Watchdog behavior at handoff (blocks M4).** (§12)

Resolve #1–#4 before attempting any on-device kernel execution.
