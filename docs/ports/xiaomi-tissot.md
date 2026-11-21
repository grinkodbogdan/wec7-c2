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
| Boot compositions | [`system/xiaomi-tissot.boot.toml`](../../system/xiaomi-tissot.boot.toml) (native drivers); [`system/xiaomi-tissot-linuxdrv.boot.toml`](../../system/xiaomi-tissot-linuxdrv.boot.toml) (plus Linux's drivers for eMMC, SD and touch) |
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
   filesystem (`rootfs = "minimal"`), and native drivers for the easy
   hardware (after QRT's, which runs on this phone):
   * `drivers/qcom/gpio-keys-tlmm`: volume up (GPIO 85) and the hall
     sensor (GPIO 44), polled every 20 ms, as `/dev/input/event0`;
   * `drivers/qcom/uart-msm`: the debug UART as a second console;
   * the kernel finds the panel's real scan-out buffer in the MDP5 pipe
     registers, and kicks command-mode panels after drawing.

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

This produces `target/images/xiaomi-tissot/` (each composition goes to
`target/images/<its name>/`):

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
firmware memory begins. The kernel is linked at 0x80080000, where LK
normally loads 64-bit kernels. If a boot loader puts it elsewhere, it
moves itself there first (checked in QEMU with
`cargo xtask fbcon-test --load-at ADDR`).
The current image is about 6.6 MiB, ending near 0x80860000.

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
* It ends at `flare:~#`. Volume up is `/dev/input/event0`; there is no
  touch input yet.
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
* **The screen keeps the fastboot or splash image.** Look at the top of
  the screen. Before the console can draw text, the kernel paints white
  stripes 32 pixels high, one per boot stage:

  | Stripes at the top | Got as far as |
  | --- | --- |
  | none | the kernel never ran, or the boot loader's screen is not at the described address (`fb0`) |
  | 1 | entered; moving itself to its link address (0x80080000) if loaded elsewhere |
  | 2 | running at its link address; turning on the MMU |
  | 3 | MMU and direct map up |
  | 4 | kernel heap up; the console starts (it reads the display controller) |
  | then text | the console is up and has cleared the stripes |

  Five more stripes lower down (rows 480–720) mean a panic before the
  console had text. A photo is the bug report.
  Black with no text can also mean a command-mode panel, which needs a
  "kick" per frame rather than continuous scan-out.
* **Text is garbled, slanted or the wrong size.** The splash buffer's
  geometry differs from the description. Adjust `fb0`'s `width`, `height`,
  `stride` and `format` (try `x8r8g8b8` with stride 4320).
* **The text stops, possibly with `KERNEL PANIC` at the bottom.** The panic
  stays on screen (`panic = "halt"`). A photo of it is the bug report.

## Linux's drivers for the hard parts

```sh
cargo xtask image --system system/xiaomi-tissot-linuxdrv.boot.toml
```

This image (`target/images/xiaomi-tissot-linuxdrv/boot.img`) also runs the
Linux driver host ([../linux-drivers.md](../linux-drivers.md)). It gives
Linux:
* the eMMC, the SD card and the touch screen's I2C bus;
* the clock controller, the pin controller, the RPM, SMEM, TCSR, the APCS
  mailbox and the PMICs, which those need.

Linux probes them with mainline's device tree for this phone
(`hardware/boards/xiaomi/tissot/linux.dtb`, as QRT ships it), trimmed to
exactly those devices (`linux.txt` there lists what else is left out). The
panel, the UART and the CPUs stay Flare's. In this composition the keys
are Linux's (it owns the pin controller), so the native key driver is not
started.

A few seconds after the shell appears, the driver host prints what Linux
found:

```
linuxdrv: Linux is up (RAM 64 MiB at ...)
linuxdrv: block devices: mmcblk0 mmcblk0boot0 ...
linuxdrv: mmcblk0: first sector read through Linux's driver: ...
linuxdrv: rtc0 (Linux's driver): ...
linuxdrv: input devices: ...
linuxdrv: waiting: <device> <what it waits for>
```

It only reads. The image is 19.8 MiB uncompressed, ending at 0x81507000,
under the 0x81800000 limit. Untested on the phone. The `waiting:` lines
say what to fix first.

## Not there yet

* the PMIC power key as a native driver (it is Linux's in the
  `-linuxdrv` composition);
* Linux's input and block devices handed to Flare (touch, eMMC storage);
* USB device mode, for `fdb`;
* display control (MDSS/DSI), so the screen can be turned off or changed;
* an MSM8953 user-space display driver, so the Arcan shell can run on the
  panel.

Only the low 2 GiB of the 4 GiB fitted is used. CPU frequency and thermal
management are left to the firmware's defaults.
