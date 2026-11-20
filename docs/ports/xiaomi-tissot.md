# Port: Xiaomi Mi A1 (tissot)

Flare's first hardware port. The Mi A1 boots the Flare framekernel from its
stock boot loader. The kernel log and the Linux personality's output (busybox,
the Linux ABI tests and droid's Android apps) are drawn on the phone's screen.

| | |
| --- | --- |
| SoC | Qualcomm MSM8953 / Snapdragon 625: 8× Cortex-A53, GICv2 |
| Panel | 5.5" 1080×1920, left on by the boot loader (continuous splash) |
| Boot loader | Qualcomm LK (Xiaomi's), Android boot image, A/B slots |
| Hardware description | [`hardware/soc/qcom/msm8953`](../../hardware/soc/qcom/msm8953/platform.fhd), [`hardware/boards/xiaomi/tissot`](../../hardware/boards/xiaomi/tissot/board.fhd) |
| Boot composition | [`system/xiaomi-tissot.boot.toml`](../../system/xiaomi-tissot.boot.toml) |
| Status | **Untested on hardware.** Built and checked as described below |

## What the port is

Following the [porting checklist](../fhd.md#porting-checklist), the port
consists of descriptions, not kernel patches:

1. **SoC layer** (`qcom/msm8953`), taken from mainline Linux's
   `msm8953.dtsi`:
   * GICv2 at 0x0b000000/0x0b002000;
   * the virtual timer on PPI 4 (INTID 20);
   * PSCI through TrustZone (SMC);
   * two clusters, CPU MPIDRs 0x0–0x3 and 0x100–0x103;
   * the debug UART and the GPIO block, described but disabled.
2. **Board layer** (`xiaomi/tissot`):
   * The memory map: usable RAM described around every firmware
     carve-out (TrustZone, qseecom, modem, ADSP, WCNSS, the boot loader,
     venus, rmtfs, ramoops), from mainline's `msm8953-xiaomi-tissot.dts`.
     The carve-outs are never mapped.
   * The panel as a `simple-framebuffer`: the boot loader's splash buffer
     at 0x90001000, 1080×1920, RGB888, stride 3240. It is the console.
   * `[boot-image]`: how LK wants the kernel packaged.
3. **Composition**: the Linux personality with the minimal root
   filesystem (`rootfs = "minimal"`). No drivers for the phone's own
   peripherals yet.

What it took in generic code (every board benefits):
* **Linux arm64 Image header.** Boot loaders that boot Linux load Flare
  the same way.
* **Fine-grained direct map.** The kernel maps memory in 2 MiB blocks and
  4 KiB pages instead of 1 GiB blocks, so firmware memory between RAM
  regions is never mapped, not even speculatively. On Qualcomm SoCs such
  an access can trip an XPU violation and reset the phone. The boot-time
  map covers only the kernel image.
* **Heap outside the image.** The kernel heap is taken from RAM at boot,
  not reserved in the image. This keeps the image small, because LK loads
  it at a fixed address with firmware memory 23 MiB above.
* **Framebuffer console.** The kernel's early console can draw on a
  `simple-framebuffer`, so a phone without an accessible UART shows its
  boot log.
* **New FHD keys:**
  * `cpu-mpidr`: per-CPU affinity values for PSCI CPU_ON;
  * `[chosen] panic = "halt"`: keep a panic on screen instead of
    powering off;
  * `[boot-image]`: how the boot loader wants the kernel packaged.
* **`cargo xtask image`** packages a kernel for a real board.

## Build

```sh
cargo xtask image --system system/xiaomi-tissot.boot.toml
```

This produces `target/images/xiaomi-tissot/`:

| File | What |
| --- | --- |
| `boot.img` | Flash or `fastboot boot` this |
| `Image` | The raw kernel (Linux arm64 Image format) |
| `board.dtb` | The device tree appended to the kernel, for LK only |

`boot.img` is an Android boot image (header v0, 2 KiB pages). It holds:
* the kernel: the Image, gzip-compressed, with a small device tree
  appended (Image.gz-dtb);
* an empty ramdisk.

LK picks an appended tree by `qcom,msm-id = <293 0>` (MSM8953) and
`qcom,board-id = <0x1000b 0>` (tissot, as in mainline). It then fills in
`/memory` and `/chosen`. Flare ignores the tree and uses the board
description compiled into the kernel.

The packer refuses images that would extend past 0x81800000, where
firmware memory begins. LK always loads 64-bit kernels at 0x80080000.
The current image is about 5 MiB, ending near 0x806a5000.

## Run it on the phone

You need an unlocked boot loader (`fastboot oem unlock`, which wipes the
phone) and the Android platform tools on your computer.

**Try it without flashing first.** `fastboot boot` runs the image once, from
RAM; the phone's storage is not touched, and a reboot brings Android back:

```sh
# Phone: power off, then hold Volume Down + Power until the FASTBOOT screen.
fastboot devices
fastboot boot target/images/xiaomi-tissot/boot.img
```

If that works and you want Flare to start at power-on, flash it to the
**inactive** slot, so Android stays on the other one:

```sh
fastboot getvar current-slot              # say it prints "a"
fastboot flash boot_b target/images/xiaomi-tissot/boot.img
fastboot --set-active=b
fastboot reboot
```

To go back: `fastboot --set-active=a` (or flash your firmware's stock
`boot.img` to `boot_b`). Fastboot mode itself is part of the boot loader,
which this never touches, so you can always reach it with Volume Down +
Power.

### What you should see

* The boot loader's unlocked-device warning, then the screen turns black.
* White text from the top: `Flare OS framekernel 0.1.0 (arm64)`, the board
  (`xiaomi-tissot` on `qcom-msm8953`), memory, `gic: GICv2 …`, the timer,
  and `smp: cpu N online` for the other cores.
* The Linux personality runs `/etc/rc`:
  * `uname`;
  * the Linux ABI tests (threads, signals, memory, events, processes,
    dynamic linking, Binder);
  * the Java and Kotlin apps under droid, with `selftest: 51/51 passed`.
* It ends at `flare:~#`. There is no keyboard or touch input yet.
  The console wraps from the bottom back to the top.
* To turn the phone off, hold Power for about 10 seconds.

The same composition runs in QEMU on a test double of this board
(`hardware/boards/qemu/virt-fbcon`):
* phone-style RAM split around a carve-out;
* a 1080×1920 RGB888 framebuffer console;
* booted as a raw Linux Image.

`cargo xtask fbcon-test` reads the framebuffer back as text and passes
when the shell prompt appears. That checks everything except the
phone-specific facts below.

### Troubleshooting

* **The boot loader goes back to fastboot ("dtb not found").** LK did not
  accept the appended device tree. Some boot loader versions may want
  other `qcom,board-id` or `qcom,pmic-id` values. Your stock `boot.img`'s
  device tree shows them (extract it with `unpack_bootimg` and
  `extract-dtb`). Put them in `[boot-image]` in `board.fhd` and rebuild.
* **The screen keeps the splash image, or stays black.** The kernel did not
  start, or crashed before drawing; nothing is logged anywhere else yet.
  Black with no text can also mean a command-mode panel, which needs a
  "kick" per frame rather than continuous scan-out.
* **Text is garbled, slanted or the wrong size.** The splash buffer's
  geometry differs from the description. Adjust `fb0`'s `width`, `height`,
  `stride` and `format` (try `x8r8g8b8` with stride 4320).
* **The text stops, possibly with `KERNEL PANIC` at the bottom.** The panic
  stays on screen (`panic = "halt"`). A photo of it is the bug report.

## Not there yet

These are drivers still to write, next in roughly this order:
* the PMIC power key and volume keys (GPIO 85);
* the debug UART (MSM UARTDM, for those with a cable on the test pads);
* USB device mode, for `fdb`;
* the touchscreen (I2C);
* eMMC storage;
* display control (MDSS/DSI), so the screen can be turned off or changed;
* an MSM8953 user-space display driver, so the Arcan shell can run on the
  panel.

Only the low 2 GiB of the 4 GiB fitted is used. CPU frequency and thermal
management are left to the firmware's defaults.
