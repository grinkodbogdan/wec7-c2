# device-info-gemini.md — values to extract for the UEFI port

Fill every `TODO`. **Pull these from gemini's own sources** (downstream Mi 5 kernel dts,
mainline `msm8996.dtsi`, postmarketOS `xiaomi-gemini`, and the running phone's partition
table). Do NOT copy from another device.

## SoC / CPU
- SoC: Qualcomm **MSM8996** (Snapdragon 820)
- CPU: 4x **Kryo** (2 "Gold" + 2 "Silver"), ARMv8.0-A  ← no LSE atomics (affects Win build choice)
- Device codename: **gemini** (Mi 5; note Mi 5s = capricorn, different panel/fingerprint)

## Memory
- DRAM base:            TODO   (mainline msm8996: commonly 0x80000000 — CONFIRM from dts)
- DRAM size:            TODO   (3GB or 4GB variant — your unit)
- cont_splash / FB carveout: TODO (addr + size — reserved-memory node)
- Other reserved/RPM/tz carveouts: TODO (must be excluded from UEFI memory map)

## UART (debug — do this FIRST)
- BLSP/UART instance wired to test pads: TODO
- UART register base:   TODO   (blsp*_uart* node in msm8996.dtsi)
- TX/RX GPIO + pinmux:  TODO   (downstream gemini dts)
- Logic level: 1.8V
- Baud to set in UART DXE: TODO (start 115200)

## Display (SimpleFb — reuse ABL cont_splash)
- Framebuffer base:     TODO
- Width x Height:       TODO (Mi 5 panel is 1080x1920)
- Stride (bytes/line):  TODO
- Pixel format / bpp:   TODO (likely BGRA 32bpp)

## Storage (UFS)
- UFS controller base:  TODO (ufshc node)
- UFS PHY base:         TODO (ufsphy node)
- Partition table (run on the phone):
  `ls -l /dev/block/bootdevice/by-name/`  → record boot/system/userdata offsets+sizes: TODO

## USB
- DWC3/XHCI base:       TODO
- USB PHY (QMP/HS) base + init: TODO

## Clocks / regulators (phase-1 minimum)
- Clocks that MUST stay on for UART+UFS+USB+FB: TODO
  (often ABL leaves these enabled; verify in UART log that they survive handoff)

## Boot packaging
- boot.img header version: TODO (match what fastboot on gemini expects)
- kernel load addr / ramdisk addr / tags addr / pagesize: TODO (from stock boot.img header,
  read with `unpack_bootimg` / `magiskboot unpack`)

## Windows build choice
- Target Windows ARM64 build: TODO (pick an OLDER build — pre-LSE-atomics requirement)
- BCD flags: start with `UseBootProcessorOnly` (single-core) until SMP bringup works
