# M2 — Development Toolchain (WEC7 Platform Builder)

> Goal: a **reproducible**, **isolated** legacy WEC7 build environment that produces a
> C2 BSP and a bootable `NK.bin`, without contaminating the main Debian host
> (Rules 1, 7; risk "Legacy WEC7 Tooling", HIGH).

## Constraints

- WEC7 Platform Builder is a **Windows-only, legacy Microsoft toolchain**. It cannot run
  natively on the Debian host.
- It must live in an **isolated virtual instance** (risk register mitigation), so the
  host stays clean and the environment is snapshot-able/reproducible.

## Required components (research + record exact versions)

| Component | Purpose | Status |
|-----------|---------|--------|
| Windows host VM (Win 7/8.1-era) | Runs Platform Builder | VERIFY exact OS version |
| Visual Studio 2008 SP1 | Host IDE for Platform Builder | VERIFY |
| Windows Embedded Compact 7 + monthly updates | Core OS + Platform Builder plug-in | VERIFY |
| WEC7 ARMv7 CEBASE / reference BSP | Scaffolding starting point (spec §5) | VERIFY |
| ARM compiler (armv7, part of PB) | Builds OAL + kernel | VERIFY |

> The exact installer set and update rollups are legacy Microsoft artifacts; record the
> precise versions and a checksum of each installer once assembled, so the environment
> can be rebuilt bit-for-bit (Rule 7).

## Isolation approach

- Build the environment inside a VM (VirtualBox/QEMU/KVM guest or equivalent) that is
  **snapshotted** immediately after a clean Platform Builder install.
- Treat the VM as disposable: everything needed to recreate it is captured as a
  documented, ordered install script + version manifest checked into
  `bsp/` tooling docs (not the VM image itself — too large, and licensing).
- The Debian host only ever holds: this repo, device backups, `fastboot`/`adb`, and
  extracted hardware dumps.

## Reproducibility manifest (to be filled)

Create `bsp/C2_SKIPJACK/BUILD.md` capturing:

- [ ] Exact OS + VS + WEC7 versions and update levels.
- [ ] Ordered install steps.
- [ ] Environment variables / Platform Builder workspace settings.
- [ ] The exact build command(s) that produce `NK.bin`.
- [ ] Output artifact locations and how to copy them to the Debian host for flashing.

## Exit criteria for M2

- [ ] Isolated VM builds the **stock ARMv7 reference BSP** to a bootable image
      (proves the toolchain works before C2 specifics are added).
- [ ] Full version manifest + install script recorded and reproducible from scratch.
- [ ] `NK.bin` output path documented and transferable to the host.

Only then does M3 (C2 BSP skeleton) begin.
