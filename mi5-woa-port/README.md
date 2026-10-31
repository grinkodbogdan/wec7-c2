# Windows on ARM — Xiaomi Mi 5 (gemini / MSM8996) porting kit

Goal of **phase 1**: get Windows ARM64 to **boot to the installer / a running desktop**
with **display, storage, and USB** working. GPU acceleration, modem, Wi-Fi, cameras,
audio, sensors are **later phases** (and GPU may never fully work — see "Hard limits").

> Status of the world (checked 2026-10): no existing Mi 5 UEFI port. Closest working
> base is the **MSM8998 (SD835)** ports. You are doing a real bringup, not a reflash.

---

## 0. Know what you're signing up for

A UEFI bringup is an **iterative on-device loop**:

```
build boot.img  ->  fastboot flash boot  ->  read UART log  ->  fix one thing  ->  repeat
```

You will do this many times. The **UART serial log is non-negotiable** — without it you
are flashing blind. Budget for a USB-to-UART cable and finding the Mi 5's debug pads.

Prereqs:
- Mi 5 (`gemini`) with **unlocked bootloader** (you have this ✅)
- A **working EDL / Mi Flash recovery path** set up BEFORE you flash anything, so a bad
  boot.img is always recoverable (fastboot boot-loop or EDL 9008). Verify you can recover
  a *known-good* stock boot.img first.
- Linux host with `fastboot`, `adb`, Python 3, and the edk2 build toolchain
- USB-TTL serial adapter (1.8V logic — **not** 3.3V/5V, you'll fry the SoC lines)

---

## 1. Fork the right base

Do **not** start from blank edk2. Start from the MSM8998 framework and retarget it:

- Framework / build system: **`edk2-porting/edk2-msm`**
- SoC-neighbor reference: **`edk2-porting/edk2-msm8998`** (SD835) — closest to 8996
- Modern Project-Mu style reference: **`edk2-porting/MU-sm8150pkg`** (how a clean device
  package is laid out; it boots Win10/11 ARM64 on newer SoCs)

MSM8996 and MSM8998 are close enough in boot flow (same ABL cont-splash display handoff,
similar UFS, similar BLSP UART) that the 8998 package is the right skeleton to copy into a
new `Msm8996Pkg` / `GeminiPkg`.

Clone:

```bash
git clone --recursive https://github.com/edk2-porting/edk2-msm
# study the 8998 device packages for structure, then create a gemini package
```

---

## 2. Extract the device-specific values (the actual hard part)

Windows-on-ARM does **not** read a Linux device tree — it reads **ACPI (DSDT/SSDT)** and a
UEFI that already set up the memory map. So you must learn the hardware from Linux sources
and firmware, then re-encode it into UEFI + ACPI.

Fill in `device-info-gemini.md` (template in this folder). Sources for each value:

| What you need | Where to get it |
|---|---|
| DRAM base + size, reserved/carveout regions | Mi 5 **downstream kernel** `arch/arm64/boot/dts/qcom/msm8996-mtp*.dtsi` + `gemini` dts; cross-check mainline `msm8996.dtsi` |
| **UART debug** base + which BLSP/GPIO | mainline `msm8996.dtsi` `blsp*_uart*` nodes; downstream `*gemini*` chooses which one is wired to the test pads |
| **Framebuffer** (cont_splash) address, width, height, stride, bpp | downstream dts `cont_splash_mem` / `framebuffer` reserved region; the ABL already inits the panel — reuse it (SimpleFbDxe style) |
| **UFS** controller base + PHY | `msm8996.dtsi` `ufshc`/`ufsphy` nodes |
| Clock/regulator (RPM) needed to keep UART+UFS alive | downstream dts; often ABL leaves these on, so phase-1 can lean on that |
| Partition layout (where `boot` is, sizes) | `gpt` of your device: `ls -l /dev/block/bootdevice/by-name/` on the running phone |

> ⚠️ Do **not** copy memory addresses from another device or from guesses. A wrong UART or
> framebuffer base = silent hang. Pull them from *gemini's own* dts.

Mainline Linux already supports MSM8996 well, so the dtsi there is a reliable map. The Mi 5
also has a mainline/postmarketOS target (`xiaomi-gemini`) — that dts is gold for GPIO/panel.

---

## 3. Build order (what to make boot, in order)

Bring these up one at a time; confirm each in the UART log before moving on:

1. **Serial / UART DXE** — first light. If you see UEFI banner text on serial, you've won
   the hardest 30%. Everything else is debuggable from here.
2. **Memory map + ACPI DSDT** — minimal DSDT describing CPUs, memory, and the fixed
   peripherals. Start from the 8998 DSDT and edit addresses to gemini's.
3. **Display (SimpleFbDxe)** — reuse the ABL cont_splash framebuffer so you get a picture
   without a real GPU driver.
4. **Storage (UFS DXE)** — needed for Windows to see the disk / installer.
5. **USB (XHCI/DWC3 device+host)** — for keyboard and for mass-storage install media.

Package the UEFI into an Android `boot.img` (UEFI as the "kernel") so fastboot can load it.
The edk2-msm build scripts produce this; point them at the gemini package.

---

## 4. Flash & test loop

```bash
# build (from edk2-msm, after adding the gemini target)
./build.sh -d gemini            # or the project's documented target name

# test WITHOUT committing to the boot partition first:
fastboot boot Gemini_UEFI.img   # loads into RAM once; safest for iteration

# only once it's stable:
fastboot flash boot Gemini_UEFI.img
```

Read serial continuously in another terminal:

```bash
# adjust device + baud (commonly 115200) to what the UART DXE sets
screen /dev/ttyUSB0 115200
```

---

## 5. Installing Windows (once UEFI boots)

- Use the WOA-Project **"WOA deployment"** flow (UUP-dump a Windows ARM64 build → apply the
  WIM to the data partition → drop a UEFI that chainloads Windows Boot Manager).
- Expect to set BCD **`UseBootProcessorOnly`** at first — the MSM8998 ports needed this
  because multi-core SMP startup wasn't wired up. MSM8996 will almost certainly need the
  same until you implement PSCI/secondary-core bringup.
- Expect to use original/neighbor **ACPI tables** edited for gemini, as the 8998 ports did.

---

## 6. Hard limits (so you aren't surprised)

- **GPU: Adreno 530 has no Windows-on-ARM driver, ever.** You will run on the software
  framebuffer. The desktop will be slow and tear. This is the ceiling, and it's why nobody
  made the Mi 5 a daily driver. Continuum-style use would be painful.
- **Modem/Wi-Fi/BT/camera/audio/sensors**: each needs its own driver effort; many never
  got done even on better-supported devices.
- **ARMv8.1 atomics**: newer Windows ARM64 builds assume LSE atomics. MSM8996's Kryo is
  ARMv8.0 — you may be pinned to **older Windows ARM64 builds** (same wall the 8998 ports
  hit). Pick an older build to start.
- **Single-core first**: see UseBootProcessorOnly above.

---

## 7. Where to ask when stuck

- `edk2-porting/edk2-msm` issues (incl. the open gemini request #388)
- WOA-Project / Project Aloha / Renegade Project communities (note: Renegade reportedly
  moved partly closed-source after code theft — mirrors may be the practical source)
- postmarketOS `xiaomi-gemini` wiki/dts — best hardware reference for this exact phone

---

### Honest bottom line

Phase 1 (serial + display + storage + USB booting Windows) is **achievable** and has been
done on the neighboring 8998. It will take real on-device work that only you can run. GPU
acceleration is the wall. Treat this as "a bootable trophy," not a usable phone — go in with
that expectation and it's a genuinely rewarding project.
