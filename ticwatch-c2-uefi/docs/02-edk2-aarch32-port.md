# 02 — EDK2 AArch32 port & bring-up

Goal: a 32-bit ARM UEFI FD that stock ABL chainloads, reaching the point where it can
emit debug (doc 03) and eventually present a UEFI shell/GOP console on the watch screen.

## 2.1 Host toolchain

```bash
sudo apt-get install -y build-essential uuid-dev iasl git nasm python3 \
    gcc-arm-linux-gnueabihf device-tree-compiler acpica-tools
```

Note the **32-bit** cross compiler: `arm-linux-gnueabihf-` (hard-float EABI), not the
aarch64 one.

## 2.2 Sources

```bash
mkdir -p ~/woa && cd ~/woa
git clone https://github.com/tianocore/edk2.git
git clone https://github.com/tianocore/edk2-platforms.git
# Pin a known-good tag; ARM (AArch32) support lives in ArmPkg/ArmPlatformPkg/MdeModulePkg.
```

EDK2 already ships the generic ARM (AArch32) building blocks you need:

- `ArmPkg` — CPU, GIC (`ArmGicLib`), architected timer, caches, exception handling.
- `ArmPlatformPkg` — the `PrePeiCore`/`PrePi` SEC/PEI entry glue for ARM platforms.
- `MdeModulePkg` — DXE core, UEFI shell, GOP console, FS stack.

You supply a thin **MSM8909W platform package** that fills in the board specifics.

## 2.3 Minimal platform package layout

Create `Platform/Qualcomm/Skipjack/` (or under a `MSM8909Pkg`):

```
MSM8909Pkg/
  Skipjack.dsc        # components + PCDs (all the doc-01 magic numbers)
  Skipjack.fdf        # FD layout: SEC, PEI, DXE, shell
  Library/
    SkipjackSerialPortLib/   # UART base from doc 01 (for DEBUG())
    SkipjackPlatformLib/     # memory map (ArmPlatformGetVirtualMemoryMap), PL/GIC bases
    SkipjackDebugLib/        # optional: screen + GPIO-blink debug (doc 03)
  Include/Skipjack.h         # #define every base address / PCD default here
```

### The values the port actually needs (from doc 01)

Expose as PCDs so there are no scattered literals:

| PCD / define                         | Source                                    |
|--------------------------------------|-------------------------------------------|
| `PcdSystemMemoryBase` (`0x80000000`) | DTS `memory@`                             |
| `PcdSystemMemorySize` (512 MB)       | DTS `memory@`                             |
| `PcdFdBaseAddress` / load addr       | stock `boot.img` base+kernel_offset       |
| GIC distributor / CPU interface base | DTS `intc` (GIC-400)                      |
| Arch timer freq (`PcdArmArchTimerFreqInHz`) | CNTFRQ / DTS `timer`               |
| UART base + clock                    | DTS `serial`/`uart` (doc 03)              |
| Framebuffer base/width/height/stride/bpp | DTS `mdss`/`cont_splash` + `/sys/.../fb0` |
| TLMM/GPIO base + debug pin           | DTS `tlmm` (doc 03)                       |
| Reserved carve-outs (TZ/modem/splash)| DTS `reserved-memory`                     |

`ArmPlatformGetVirtualMemoryMap()` in `SkipjackPlatformLib` must describe DRAM,
device MMIO, and every reserved carve-out exactly, or UEFI faults before it can tell
you why. (That's what doc 03's debug channels are for.)

## 2.4 Build

```bash
cd ~/woa/edk2
. edksetup.sh
export GCC5_ARM_PREFIX=arm-linux-gnueabihf-
export PACKAGES_PATH=$PWD:$PWD/../edk2-platforms

# DEBUG build, 32-bit ARM:
build -a ARM -t GCC5 -b DEBUG -p MSM8909Pkg/Skipjack.dsc
```

Output: an FD (e.g. `SKIPJACK_EFI.fd`). Wrap for ABL using the offsets from the stock
`boot.img` (doc 01):

```bash
mkbootimg \
  --kernel SKIPJACK_EFI.fd \
  --base <from stock boot.img> \
  --kernel_offset <from stock> \
  --tags_offset <from stock> \
  --pagesize <from stock> \
  --output uefi-boot.img
```

## 2.5 Test loop (safest first)

```bash
fastboot boot uefi-boot.img     # volatile — nothing written to flash
```

Black screen on first attempts is normal. **Do not** iterate blindly — attach a debug
channel (doc 03) so each attempt tells you how far it got.

## 2.6 Bring-up order

1. SEC/PEI entry survives, UEFI relocates into valid DRAM.
2. **A debug signal** — framebuffer paint or GPIO blink (doc 03). *Before anything else.*
3. Architected timer + GIC (needed for DXE and any delays).
4. UART `SerialPortLib` → real `DEBUG()` output and the UEFI console.
5. GOP over the existing framebuffer → UEFI shell visible on the round display.
6. Storage (eMMC) + USB (if the watch exposes it) for loading payloads.
7. (Optional) a tiny boot menu / shell as the end deliverable.

Because there's no ARM64 Windows target, GOP + UEFI shell on the 360×360 screen with
working UART is a reasonable "done" line for this device.
