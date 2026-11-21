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
  | 1 | entered; moving itself to its link address (0x80080000) if loaded elsewhere (a 20 MiB image takes about 2 s) |
  | 2 | running at its link address; turning on the MMU |
  | 3 | MMU on, running Rust in the upper half |
  | 4 | exception vectors installed; building the kernel's page tables |
  | 5 | page tables built; switching to them |
  | 6 | switched; setting up the kernel heap |
  | 7 | heap up; the console starts (it reads the display controller) |
  | then text | the console is up and has cleared the stripes |

  Five more stripes lower down (rows 480–720) mean a panic before the
  console had text. `tools/bootimg/early-boot.sh` runs this kernel's early
  boot on QEMU, loaded at LK's address, and prints the screen up to the
  first SoC device access. It reproduces such panics with their message. A photo is the bug report.
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

This is the full phone. It is two images, because one did not fit
(`fastboot` failed sending 112 MB with "too many links"):

| File | Size | What | Where |
| --- | --- | --- | --- |
| `boot.img` | ~9 MB | the kernel, the minimal root filesystem, the Linux driver host | a boot slot |
| `userdata.img` | ~48 MB | the full root filesystem (Arcan, Mesa, the shell, ART), compressed | the `userdata` partition |

```sh
fastboot flash userdata target/images/xiaomi-tissot-linuxdrv/userdata.img   # erases Android's data
fastboot flash boot_b target/images/xiaomi-tissot-linuxdrv/boot.img
fastboot --set-active=b && fastboot reboot
# or try it once from RAM, after flashing userdata:
fastboot boot target/images/xiaomi-tissot-linuxdrv/boot.img
```

What runs:

* **Storage.** Linux's eMMC driver finds the GPT partition named
  `userdata` and reads the root filesystem (46 MB compressed) into memory
  the driver host owns. The kernel checks it, inflates it (zlib,
  `miniz_oxide`) and adds its files (`[linux] rootfs = "userdata"`,
  `FLAREHW_ROOTFS`). Then the shell starts.
* **Display** (`drivers/qcom/mdp5-display`): the boot loader's display
  setup stays. One source pipe of the display engine is re-pointed from the
  24-bit splash buffer to the `display` shared memory as 32-bit XRGB8888
  (QRT's recipe for this phone). That memory is `/dev/dri/card0` for Linux
  programs. The kernel console moves onto it too, and steps aside once the
  shell presents frames.
* **Touch and keys**: Linux's drivers (ft5x06, gpio-keys, the PMIC's power
  key) in the driver host, handed to Flare (`[linux] inputs`) as
  `/dev/input/event0` (the touch screen, as an absolute pointer in
  0..32767) and `event1` (all keys).
* **fdb over USB**: Linux's DWC3 controller in device mode with its serial
  gadget, carrying Flare's `/dev/fdb` (`[linux] serials`). On the computer:
  `FDB_SOCKET=/dev/ttyACM0 cargo fdb shell`.
* **Quick boot**: the driver host first; the shell starts once the root
  filesystem, touch and keys are there.
* **Arcan's watchdog** kills its engine (SIGUSR1) when a frame takes more
  than 5 s, and on the phone's 8 cores and 1080×1920 panel the first,
  software-rendered frame did, before the engine had a handler. Arcan ended
  silently. Found by reproducing with 8 CPUs in QEMU; the timeout is now
  `ARCAN_WATCHDOG_TIMEOUT_MS` (compat/arcan/patches/0005), 60 s in
  `flare-ui`. The kernel now reports signal deaths and faults sent to a
  handler, and `/proc/flare-threads` lists what every thread waits in. If
  the shell is not up after 30 s, `/etc/rc` prints that on the screen.

The GPU is next: [../gpu.md](../gpu.md).

The driver host also gives Linux:
* the eMMC, the SD card and the touch screen's I2C bus;
* the clock controller, the pin controller, the RPM, SMEM, TCSR, the APCS
  mailbox and the PMICs, which those need.

Linux probes them with mainline's device tree for this phone
(`hardware/boards/xiaomi/tissot/linux.dtb`, as QRT ships it), trimmed to
exactly those devices (`linux.txt` there lists what else is left out). The
panel, the UART and the CPUs stay Flare's. In this composition the keys
are Linux's (it owns the pin controller), so the native key driver is not
started.

Once Linux's devices have settled (up to 30 s after the shell appears), the
driver host prints what Linux found:

```
linuxdrv: Linux is up (RAM 64 MiB at ...)
linuxdrv: block devices: mmcblk0 ... (and N partitions)
linuxdrv: mmcblk0: first sector read through Linux's driver: ...
linuxdrv: mmcblk0: 8 of 8 uncached reads (one device interrupt each)
linuxdrv: rtc0 (Linux's driver): ...
linuxdrv: input devices: event0 (...), event1 (...), ...
linuxdrv: waiting: <device> <what it waits for>
```

After that it prints each key press and each new touch
(`linuxdrv: <device>: touch 1 at x,y`). It only reads storage; nothing is
written. `waiting:` lines say what to fix next.

What it took on the phone (and from QRT's notes on bringing up this phone
with LKL):

* **The RPM's replies are edge interrupts.** The kernel had masked every
  interrupt it gave the driver host until Linux unmasked it. Linux's edge
  flow never unmasks, so the RPM's second reply was lost and its clocks
  timed out (-110).
* **OPP support** for the RPM's power domains (`PM_OPP`, which only another
  option can select). Without it, `qcom-rpmpd` failed (-95). The clock
  controller sits in its CX domain, so the clock controller and everything
  after it (eMMC, I2C, DMA) waited (QRT 0.21.5). The LKL build now checks
  that every option asked for was set.
* **No secure-world calls** (QRT 0.21.10, 0.21.14): `/firmware` (SCM) is
  dropped from Linux's tree, and the composition allows no SMC calls.
* **No native UART driver** in this composition: the UART shares BLSP1's
  bus clock with the I2C controllers, which Linux switches off when idle
  (QRT 0.21.16).
* **No interconnect paths.** The bus (NoC) drivers are not Linux's, and
  the RPM keeps the boot loader's bus clocks. Linux's tree has no
  `interconnects` references and no bandwidths in its OPP tables, so the
  SDHC (eMMC, SD) does not wait for providers that never appear.
* `/firmware` is disabled in the whole subtree: Linux creates `/firmware`'s
  children whatever the parent's status says.
* **Quick boot** (`FLARE_BOOT=quick` in the composition): `/etc/rc` starts
  the driver host first and skips the test suites; Linux shows only critical
  messages (`loglevel=3`). The report lists anything still waiting.
* `fw_devlink=permissive`, `clk_ignore_unused`, `pd_ignore_unused`,
  `regulator_ignore_unused`. QRT's LKL patches keep the boot loader's RPM
  clocks and power-domain corners.

## Not there yet

* the PMIC power key as a native driver (it is Linux's in the
  `-linuxdrv` composition);
* writable storage: the eMMC is read (the root filesystem) but not yet a
  Flare block device or file system for programs;
* display control (MDSS/DSI), so the screen can be turned off or changed;
* the GPU (Adreno 506): the shell renders in software (Mesa softpipe) at
  1080×1920. The plan: [../gpu.md](../gpu.md).

Only the low 2 GiB of the 4 GiB fitted is used. CPU frequency and thermal
management are left to the firmware's defaults.
