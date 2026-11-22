# Drivers: native and Linux's own

Flare drives hardware two ways.

* **Easy hardware gets a native driver**: a small `no_std` component
  holding exactly the capabilities its composition grants, e.g. a UART, a
  GPIO key or an RTC. Examples: `drivers/qcom/uart-msm`,
  `drivers/qcom/gpio-keys-tlmm`, `drivers/common/*`.
* **Hard and expensive hardware runs Linux's own driver**: eMMC
  controllers, PMICs and their regulators, clock trees, I2C touch screens,
  USB, later display and GPU. These are tens of thousands of lines tied to
  Linux's clock, power-domain, pinctrl and regulator frameworks. Instead of
  rewriting them, Flare runs Linux itself as a library in a process: the
  **driver host**.

This follows QRT (github.com/grinkodbogdan/qrt), which drives the Xiaomi
Mi A1 with Linux's drivers through LKL (the Linux Kernel Library). The
difference is that QRT gives Linux the whole machine. Flare gives the
driver host only what its composition lists.

## The driver host

`compat/linux/linuxdrv` builds one static Linux program, `/usr/bin/linuxdrv`:

* LKL ([github.com/lkl/linux](https://github.com/lkl/linux), Linux 6.12)
  at the commit QRT uses, with:
  * QRT's patches (`lkl/qrt.patch`): device tree boot of a real SoC
    (`LKL_QRT_SOC`), the GIC, LKL's MMU mode, non-coherent DMA, and its
    Qualcomm driver fixes;
  * Flare's (`lkl/flare.patch`): interrupt control and SMC calls go
    through host operations, because a user process can own neither.
* `lkl/config-arm64`: QRT's options cut down to the SoC basics, input and
  storage. `lkl/config-flare`: QEMU virt's devices and the PMIC RTC.
* `linuxdrv.c`: the host operations (threads, timers, memory, register
  access, interrupts) over `/dev/flarehw`.

It starts Linux with the board's device tree, mounts devtmpfs, sysfs and
debugfs inside Linux, and reports what Linux found:
* block devices, with each one's first sector read through Linux's driver;
* the RTC;
* input devices;
* devices still waiting for a dependency (`devices_deferred`, which is
  what to look at when bringing up a board).

`cargo xtask build` builds it when the composition gives Linux hardware.
The first build takes about 10 minutes on 4 cores; later builds compile
only what changed. `/etc/rc` starts it in the background.

## Giving hardware to Linux

```toml
[linux]
hardware = ["virtio31", "rtc0"]   # FHD devices: their registers and interrupts
smc = ["0x84000000..0x8400001f"]  # SMC Calling Convention function ids it may call
```

The build refuses a composition in which any of the following holds:
* a device named here is unknown or disabled;
* a device is also granted to a component;
* a device's registers share a page with registers granted to a component.
  For example, the pin controller cannot go to Linux while the native key
  driver holds one of its pins.

The kernel serves the grant as `/dev/flarehw`, present only when
`hardware` is non-empty:

| | |
| --- | --- |
| `ioctl(0x4600, {index})` | device `index`: name, register windows, interrupts |
| `mmap(offset = physical address)` | a granted register window as Device memory (Normal non-cacheable for `mapping = "memory"` devices such as SMEM), or DMA memory this process allocated (cached; offset bit 62: non-cacheable). Anything else: `EPERM` |
| `ioctl(0x4601, {size})` | physically contiguous DMA memory; returns its physical address |
| `ioctl(0x4602, {intid, op, arg})` | 0: GIC lines; 1/2: unmask/mask; 3: edge or level. Granted interrupts only |
| `read()` | the INTIDs that fired, 4 bytes each. A level-triggered line stays masked until unmasked (Linux's level flow does that); an edge-triggered one is not masked, since Linux's edge flow never unmasks (Qualcomm's RPM replies are edges) |
| `ioctl(0x4603, regs[8])` | an SMC call whose function id is allowed |

Linux's RAM (64 MiB) is DMA memory mapped at LKL's fixed address. Physical
addresses given to devices are therefore right, with no IOMMU needed.

## Device trees

* **Virtual boards** describe their devices in FHD only. The build turns
  the granted devices into a device tree (`tools/bootimg/fhd2dtb.py`).
* **Real boards** ship their mainline Linux device tree as
  `hardware/boards/<board>/linux.dtb`. At build time,
  `tools/bootimg/dtb.py trim` disables every device whose registers were
  not granted, plus the nodes listed as `drop` in `linux.txt` next to it,
  so Linux probes only its share. `linux.txt` also holds kernel arguments
  (`cmdline`). `dtb.py info` lists a tree's devices, windows and
  interrupts, which is how the FHD entries for Linux's devices are written.

## The whole device tree (`devicetree = true`)

```toml
[linux]
devicetree = true                 # every device in the board's linux.dtb
smc = ["0x02000000..0x02ffffff", "0x42000000..0x42ffffff"]  # Qualcomm SCM (SiP) calls
```

With `devicetree`, Linux gets its board's whole tree, as on a Linux phone.
`dtb.py grants` lists every register window and GIC interrupt the
remaining nodes use. The kernel grants them, minus what components hold
(a page shared with a component's registers, a component's interrupt) and
Flare's own RAM. The build says what it left out. Reserved memory
(firmware images, SMEM) is granted as memory, not registers. The board's
`linux.txt` decides the rest:

* `drop PATH`: Linux never sees the node (the CPUs' own hardware, the
  panel, what can reset the phone);
* `hide PATH...`: one stage. Linux sees the nodes only once the driver host
  has their firmware ready: their compatibles start with a capital
  (`Qcom,adreno`) in the
  tree until then.

After the phone shell is up, `linuxdrv`:
1. mounts the phone's partitions read-only inside Linux (`/mnt/modem_a`,
   `/mnt/vendor_a`, `/mnt/persist`);
2. lays out the firmware: Linux's search path is the modem partition's
   `image/` (the Wi-Fi and DSP firmware). The GPU's microcode and zap
   shader, and Wi-Fi's calibration, are copied from vendor/persist to
   `/lib/firmware`;
3. runs each stage: it restores the compatibles in the tree Linux reads
   (Linux's properties point into the host's buffer), then probes every
   device that has no driver, then starts the remote processors waiting
   (`/sys/class/remoteproc/*/state`);
4. asks the GPU for its id and clock (`DRM_IOCTL_MSM_GET_PARAM`), and
   prints what Linux drives: backlight, power supplies, network
   interfaces, sound cards, sensors, input, and what still waits.

The driver set is QRT's for the Mi A1 (`lkl/config-qrt`), with the options
the phone needs checked in `lkl/config-flare`.

## Linux's devices for every program

What the driver host's Linux drives is the whole system's
(`kernel/src/linux/remote.rs`, served by `linuxdrv` over `/dev/flarelk`):

| In Flare | Is the driver host's |
| --- | --- |
| `/linux/PATH` | its `PATH`: `/linux/sys/...`, `/linux/dev/...`, `/linux/proc/...` |
| `/sys/class/power_supply`, `backlight`, `leds`, `net`, `rfkill` | its sysfs classes (battery, brightness, LEDs, network interfaces) |
| `/proc/net` | its network tables (`ifconfig`, `route`, `netstat` read them) |
| `/dev/snd` | its sound cards (ALSA) |
| sockets: IPv4, IPv6, netlink, packet | its network stack: Wi-Fi, DHCP, DNS, TCP and UDP |

A system call on one of these becomes a request; the driver host makes the
same call on its Linux, in a thread of its own (calls may block), and
replies. The caller waits meanwhile. No pointer crosses: the kernel copies
what a call reads and writes:
* ioctls by the size their number encodes; interface ioctls as `struct
  ifreq`;
* ALSA's frame transfers (`WRITEI`/`READI`) by the stream's frame size,
  taken from its `HW_PARAMS`;
* socket filters (`SO_ATTACH_FILTER`) as their instructions;
* `sendmsg`/`recvmsg` gathered and scattered (no control messages).

The host reports every descriptor's readiness, so `poll`, `epoll` and
`select` work on them. Unchanged programs use it: busybox's `udhcpc`,
`ifconfig`, `route`, `wget` and `nslookup`, and `wpa_supplicant` and
`wpa_cli` from Alpine (`compat/linux/wifi`). On top:
* `wifi scan`, `wifi connect SSID [PASSWORD]`, `wifi status`, `wifi off`:
  wpa_supplicant (nl80211) and DHCP;
* `brightness [PERCENT]`;
* `flare-statusd`: the shell's status bar shows the battery (and charging)
  and the Wi-Fi network.

`cargo xtask linuxdrv-test` checks it end to end on QEMU. The driver host
drives a virtio network card there. A Flare program reads its `/proc` and
`/sys`, gets an address by DHCP and fetches a page from the host over TCP.

Not through this yet: the GPU. Its ioctls carry pointers to tables and
its buffers must be mapped, not copied ([gpu.md](gpu.md)).

## Adding a driver

Any driver in Linux's tree can run in the driver host. On a real board:

With `devicetree = true` (the Mi A1), a driver in Linux's tree needs only
steps 3 and 4. Otherwise:

1. **The device**: `python3 tools/bootimg/dtb.py fhd hardware/boards/<board>/linux.dtb <node> <name>`
   prints its FHD entry from mainline's device tree (compatible, register
   windows, GIC interrupts as INTIDs). Paste it into the SoC's
   `platform.fhd`, and enable it in the board's `board.fhd`.
2. **The grant**: add `<name>` to the composition's `[linux] hardware`.
   The build checks it against everything Flare's own drivers hold.
3. **The node**: make sure `linux.txt` does not `drop` it (or its parent).
4. **The driver**: its `CONFIG_` option in
   `compat/linux/linuxdrv/lkl/config-flare` (the build fails if Kconfig
   drops it). QRT's `ports/lkl/config-arm64` is a known-good set for the Mi
   A1: GPU, Wi-Fi, Bluetooth, audio, sensors, fuel gauge, backlight and
   vibrator.

Devices on buses Linux owns (I2C, SPMI, GPIO interrupts) need no grant of
their own: their controller's grant covers them.

**Why not a `.ko`?** A module is machine code for one exact kernel: one
architecture, configuration, structure layout and set of symbol versions
(`vermagic`, `modversions`). The driver host's Linux is LKL, which is
architecture `lkl`, with its own configuration, so a module built for an
arm64 phone kernel cannot load into it, any more than into another arm64
kernel built differently. LKL also has no module loader. Drivers are built
into the driver host from source instead; a driver that is in Linux's tree
needs only the steps above. An out-of-tree driver's source goes next to
the LKL patches and is built the same way.

## Tested

`cargo xtask linuxdrv-test` boots `system/qemu-virt-linuxdrv.boot.toml`,
the phone image's shape on QEMU. The reference phone board's disk, RTC,
tablet and keyboard go to Linux instead of Flare's native drivers; the
host hands the tablet and keyboard back as Flare's `touch` and `keys`.
There is a display driver and the full root filesystem, booted the quick
way into the phone shell. It checks that:
* the driver host gets exactly those two devices;
* Linux's virtio-blk driver reads the userdata disk's first sector (it
  starts with `FLAR`); this needs DMA and completion interrupts;
* with its interrupt edge-triggered (`linux-irq = "edge"`), eight uncached
  reads, each completed by an interrupt;
* Linux's PL031 driver reads the clock;
* a key pressed on QEMU's keyboard reaches Flare's `/dev/input/event1`
  through Linux's driver (the bytes are checked);
* the phone shell starts and is unlocked by a swipe through Linux's
  tablet driver.

Anything else Linux tries to map is refused and logged
(`linuxdrv: ioremap(...): not hardware this host was given`).

## On the Xiaomi Mi A1

`system/xiaomi-tissot-linuxdrv.boot.toml` gives Linux:
* the eMMC (SDHC1), the SD card (SDHC2) and the touch screen's I2C bus;
* what those depend on: the global clock controller, the pin controller,
  the RPM (power rails), SMEM, TCSR, the APCS mailbox, and the PMICs over
  SPMI.

The panel, the UART and the CPUs stay Flare's. The tree is mainline's
`msm8953-xiaomi-tissot.dts`, trimmed as above. `clk_ignore_unused`,
`pd_ignore_unused` and `regulator_ignore_unused` keep Linux from turning
off the panel's clocks and rails, which it does not know are in use. See
[ports/xiaomi-tissot.md](ports/xiaomi-tissot.md).

## Input devices for Flare

`[linux] inputs = ["touch", "keys"]` names Flare input devices
(`/dev/input/eventN`) that the driver host feeds, instead of an input
driver component. `ioctl(FLAREHW_INPUT, {index, op, ptr, len})` with:
* op 2: the device's name;
* op 0: describe it (`flare_abi::proto::input::Describe`);
* op 1: report events.

The host describes each Linux input device from Linux's own evdev
(name, ids, capability bits, axis ranges). It gives the touch screen to the
Flare device whose name contains `touch`, as an absolute pointer in
0..32767 with `BTN_LEFT` (as the phone shell takes touch). All key devices
go to the one whose name contains `key`. Then it writes
`/tmp/linuxdrv.ready`, which `/etc/rc` waits for before starting a user
interface.

## Storage and serial devices for Flare

* `[linux] rootfs = "userdata"`: the full root filesystem is on storage,
  in the format of `kernel/src/ramdisk.rs` (zlib-compressed). The host
  reads it through Linux's block driver from the GPT partition named
  `userdata` (or, in QEMU, a disk that is the image). It reads it into its
  own DMA memory, and `ioctl(FLAREHW_ROOTFS, {phys, len})` has the kernel
  check it, inflate it into fresh memory and add its files.
* `[linux] serials = ["fdb"]`: a Flare serial device carried over a tty
  of the host's Linux: the USB serial gadget (`ttyGS0`) on a phone, the
  virtio console (`hvc0`) in QEMU. `ioctl(FLAREHW_SERIAL, {index, op,
  ptr, len})` with op 0 (bytes for Linux programs) or op 1 (what they
  wrote). The test runs fdb's checks over it.

## Not yet

* The GPU, through Linux's own driver: [gpu.md](gpu.md).
* Linux's block devices as writable Flare storage (today the host reads
  the root filesystem from them). Next is bridging them to Flare:
  * block devices to the storage endpoint, so the Linux personality and
    components can use the eMMC.
* USB device mode (for `fdb`), display and GPU, with the IOMMU and the
  firmware loading they need.
* LKL runs on one CPU; interrupts are delivered through a thread.
