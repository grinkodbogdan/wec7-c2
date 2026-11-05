# UEFI for the Orange Pi Zero 2W (Allwinner H618)

An EDK2 (TianoCore) UEFI firmware port for the Orange Pi Zero 2W. This is
the first step towards running Windows 11 on ARM, or any other UEFI + ACPI
operating system, on the board.

> **Status: stage 1, not yet tested on hardware.** The ACPI tables have been
> compiled and decoded with `iasl`, and the board code compiles cleanly for
> AArch64. The full firmware has not been built or booted yet.

## What works (by design) in stage 1

| Feature | State |
|---|---|
| Boot chain SPL → TF-A → UEFI | Implemented |
| DRAM size detection (1 / 1.5 / 2 / 4 GiB) | Implemented |
| Serial console on UART0, 115200 8N1 | Implemented |
| UEFI Shell and boot menu (over serial) | Implemented |
| ACPI: FADT, MADT, GTDT, SPCR, DBG2, DSDT | Implemented |
| PSCI reset/power-off and SMP (via TF-A) | Implemented |
| UEFI variables | In RAM only, lost at reboot |
| SD card / eMMC | **Not yet**: needs an Allwinner MMC driver |
| USB | **Not yet** |
| HDMI (GOP) | **Not yet** |
| Wi-Fi / Bluetooth | **Not yet** |
| SMBIOS tables | **Not yet** |

Without a storage driver, stage 1 can only reach the UEFI Shell. It cannot
load an OS yet.

## How it boots

```
Boot ROM ─► U-Boot SPL ───────────► TF-A BL31 ─────────────► EDK2 UEFI
            (inits DRAM, loads      (secure monitor, PSCI)    (this port, entered
             the FIT from SD)                                  at EL2, MMU off)
```

Mainline U-Boot SPL already knows how to set up the Zero 2W's LPDDR4, so it
is reused as-is. Only U-Boot proper is replaced: the FIT that SPL loads holds
this firmware in place of `u-boot-nodtb.bin` (see `boot/uefi.its`).

### Memory map

| Range | Use |
|---|---|
| `0x0000_0000`–`0x3FFF_FFFF` | SRAM and MMIO |
| `0x4000_0000`–`0x401F_FFFF` | TF-A BL31 (reserved for the OS too) |
| `0x4A00_0000`–`0x4A2F_FFFF` | This firmware (freed at ExitBootServices) |
| `0x7C00_0000`–`0x7FFF_FFFF` | UEFI stack and HOBs during early boot |
| `0x8000_0000`–end of DRAM | Extra DRAM, size probed at boot |

## Building

On Ubuntu 24.04 or Debian 12:

```sh
sudo apt install git make gcc gcc-aarch64-linux-gnu python3 python3-setuptools \
  python3-pyelftools uuid-dev acpica-tools device-tree-compiler bison flex \
  libssl-dev bc swig libgnutls28-dev
./build.sh            # DEBUG build, verbose log on serial
./build.sh RELEASE    # quiet build
```

The script fetches pinned versions of EDK2 (`edk2-stable202608`), TF-A
(`v2.15.0`) and U-Boot (`v2026.10`) into `work/`, builds all three, and
writes `out/opiz2w-uefi-sunxi-with-spl.bin`.

## Flashing and first boot

1. Use a **spare** microSD card. Writing overwrites the first few MiB:
   ```sh
   sudo dd if=out/opiz2w-uefi-sunxi-with-spl.bin of=/dev/sdX bs=1024 seek=8 conv=fsync
   ```
2. Connect a 3.3 V USB-serial adapter to the debug UART on the 40-pin header
   (GND, UART0 TX = PH0, UART0 RX = PH1; check your board's pinout) and open it
   at 115200 8N1.
3. Insert the card and power on. You should see U-Boot SPL's DRAM message,
   then TF-A's `NOTICE:` lines, then the EDK2 log, then a 5-second boot menu
   countdown and the UEFI Shell.

If it stops after TF-A, the firmware did not start. A DEBUG build log up to
the hang is the most useful thing for debugging.

## Roadmap

1. **Boot to the UEFI Shell on real hardware** (this stage).
2. **SD card driver** for the Allwinner MMC controller, so `.efi` files and
   OS loaders can be started from a FAT partition.
3. **Persistent UEFI variables** stored on the SD card.
4. **USB host** (EHCI/OHCI through EDK2's NonDiscoverable device support),
   for a keyboard and USB storage.
5. **SMBIOS tables**, which Windows expects.
6. **HDMI framebuffer (GOP)**, needed for Windows' boot screen.
7. **Windows 11 on ARM, version 23H2 or older.** 24H2 and later need ARMv8.1
   atomics, which the Cortex-A53 (ARMv8.0) does not have. Windows will also
   need its own drivers for the SD controller, USB and display, and none
   exist yet.

## Layout

```
build.sh                         Fetch + build everything
boot/uefi.its                    FIT description loaded by U-Boot SPL
OrangePiZero2W/
  OrangePiZero2W.dsc / .fdf      EDK2 platform description and flash layout
  OrangePiZero2W.dec             Package and board PCDs
  Include/H618.h                 SoC addresses and interrupt numbers
  Library/PlatformLib/           ArmPlatformLib: cores, memory map, DRAM probe
  Library/MemoryInitPeiLib/      Memory HOBs + MMU setup
  AcpiTables/                    FADT, MADT, GTDT, SPCR, DBG2, DSDT
```

Licensed BSD-2-Clause-Patent, like EDK2. Parts are modelled on the Raspberry
Pi port in [edk2-platforms](https://github.com/tianocore/edk2-platforms).
