# 01 — Extract the `skipjack` hardware map

This is the binding spec for everything downstream. Do it first. Because the C2 already
runs mainline-style Linux, you can read nearly every value you need instead of guessing.

## 1.1 Sources

| Source                     | What you get                                             |
|----------------------------|----------------------------------------------------------|
| AsteroidOS `skipjack` DTS  | memory map, GIC, timer, TLMM/GPIO, clock (GCC), SPMI/PMIC, display |
| Void Linux live on device  | `dmesg`, `/proc/iomem`, `/proc/interrupts`, `/sys/...`   |
| Qualcomm MSM8909 docs       | register meanings, clock IDs, pin mux                    |

> MSM8909W ≈ MSM8909 with the wearable package. Most MSM8909 register definitions apply;
> confirm anything power/clock-related against the `skipjack` DTS.

## 1.2 Pull the device tree

```bash
# From the AsteroidOS / mainline kernel tree for skipjack:
#   arch/arm/boot/dts/qcom/*8909*skipjack*.dts (name varies)
# Decompile the running DTB to be sure it matches the unit:
adb shell su -c 'cat /sys/firmware/fdt' > skipjack.dtb   # if exposed, else pull the boot dtb
dtc -I dtb -O dts skipjack.dtb -o skipjack.dts
```

Record these nodes/values into `docs/hardware-map.md` (create it as you go):

- `memory@...` — DRAM base and size (expect base `0x80000000`, 512 MB).
- `reserved-memory { ... }` — regions you must NOT use (TZ, modem/RPM, splash fb,
  SMEM). Each becomes a carve-out in your UEFI memory map.
- `intc` / GIC — distributor + CPU interface base (MSM8909 uses GIC-400 / GICv2).
- `timer` — the ARM architected timer (CNTFRQ) and/or the Qualcomm MSM timer base.
- `tlmm` / `gpio` — pin controller base + the pins used for display reset/backlight,
  touch INT, vibrator, buttons. You'll reuse some as debug "blink" channels.
- `spmi` + `pm8909`/PMIC — SPMI base and the PMIC peripheral addresses (regulators,
  LED/backlight drivers). 32-bit WoA-style ports still need PMIC for display power.
- Display pipeline (`mdss`/`mdp`/`dsi`) — needed later; the **framebuffer base** the
  bootloader hands over is what you paint first (see `docs/03`).
- Clocks (`gcc` — global clock controller base) — you re-enable clocks UEFI needs
  (UART, display) that Linux used to turn on.

## 1.3 Pull the live runtime state

```bash
adb shell su -c 'cat /proc/iomem'        > iomem.txt       # authoritative mmio map
adb shell su -c 'cat /proc/interrupts'   > interrupts.txt  # IRQ numbers
adb shell su -c 'dmesg'                  > dmesg.txt        # clock/regulator/fb bring-up order
adb shell su -c 'cat /proc/cmdline'      > cmdline.txt      # bootloader-passed args incl. fb
# Framebuffer geometry the bootloader set up (continuous splash):
adb shell su -c 'cat /sys/class/graphics/fb0/virtual_size' 2>/dev/null
adb shell su -c 'cat /sys/class/graphics/fb0/stride'       2>/dev/null
adb shell su -c 'cat /sys/class/graphics/fb0/bits_per_pixel' 2>/dev/null
```

## 1.4 Boot chain (MSM8909W)

```
PBL → SBL1 → QSEE/TZ + RPM → ABL (LK/aboot) → boot.img (kernel) OR your UEFI payload
```

Same strategy as any Qualcomm fastboot device: wrap your EDK2 FD as an Android
`boot.img` so stock ABL chainloads it at the kernel load address. Get the load/base
offsets from the stock `boot.img`:

```bash
# Back up first (see rule 2), then inspect the stock boot image:
abootimg -i boot.img          # or: unpackbootimg -i boot.img
#   note: base, kernel_offset, tags_offset, pagesize, cmdline
```

## 1.5 Recovery path — set this up BEFORE flashing anything

```bash
# Full raw backups of the partitions you might touch:
adb shell su -c 'dd if=/dev/block/bootdevice/by-name/boot     of=/sdcard/boot.bak.img'
adb shell su -c 'dd if=/dev/block/bootdevice/by-name/recovery of=/sdcard/recovery.bak.img'
# pull them off-device, and keep the stock fastboot/EDL (9008) restore images.
```

Prefer **temporary boot** over flashing for every early test:

```bash
fastboot boot uefi-boot.img    # runs it once, nothing written — the safest test loop
```

Deliverable of this doc: a complete `hardware-map.md` with every base address, IRQ,
GPIO, clock ID and the framebuffer geometry. Nothing in `docs/02`/`docs/03` works
without it.
