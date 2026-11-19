# Flare Hardware Description (FHD), version 0

A port **describes** hardware; it does not patch the kernel. FHD is the
machine-readable description of a board, split into layers:

| Layer | File | Holds |
| --- | --- | --- |
| SoC | `hardware/soc/<vendor>/<soc>/platform.fhd` | CPUs, memory windows, interrupt controller, timers, firmware interface, on-chip peripherals — everything true of every board using the chip |
| Board | `hardware/boards/<vendor>/<board>/board.fhd` | Which SoC, fitted memory, which peripherals are enabled, board wiring and roles, console, emulator settings |

The board layer is merged over the SoC layer **key by key**, so a board only
states what differs (`status = "okay"`, a smaller `size`, a `role`). Every
value keeps its origin (file and line) for error messages.

## Syntax

The shared `flare-config` syntax: TOML-like sections and `key = value`
pairs; values are strings, integers (`0x0900_0000`), booleans or arrays.
Duplicate sections or keys are errors.

```toml
fhd = 0                    # format version
kind = "soc"               # or "board"
name = "qemu-virt"
arch = "arm64"             # SoC layer only
cpus = 4

[memory.dram]              # memory regions: base, size, reserved = bool
base = 0x40000000
size = 0x40000000

[intc.gic]                 # hardware nodes: [<class>.<name>]
compatible = ["arm,gic-v2", "arm,cortex-a15-gic"]   # most specific first
reg = [[0x08000000, 0x10000], [0x08010000, 0x10000]]

[device.uart0]
compatible = ["arm,pl011", "arm,primecell"]
reg = [0x09000000, 0x1000]
interrupts = [33]          # interrupt-controller-native ids (GIC INTIDs)
status = "okay"            # or "disabled"
requires = []              # nodes this one depends on (regulators, clocks…)
clock-hz = 24000000        # any other key is a property

[chosen]
console = "uart0"

[emulation]                # virtual boards only
qemu-machine = "virt"
qemu-cpu = "cortex-a53"
memory-mib = 256
```

A board names its SoC with `soc = "<vendor>/<soc>"`. Node classes in use:
`intc`, `timer`, `firmware`, `device`; new classes (`clock`, `regulator`,
`pinctrl`, `power-domain`, `iommu`, `bus`) follow the same shape.

## Validation

`flare_fhd::validate` reports every problem with file and line:

* unsupported format version or architecture; unknown keys; wrong kinds
* empty or overflowing register windows; **overlapping** register windows
  or memory regions
* not exactly one enabled interrupt controller; no timer with an interrupt
* an interrupt used by two enabled nodes (unless `shared-irq = true`)
* `requires` naming unknown or disabled nodes; `chosen.console` naming an
  unknown or disabled node
* `[emulation] memory-mib` disagreeing with the described RAM

`cargo xtask fhd <vendor>/<board>` validates and prints a board.

## Consumers

* **Kernel build** (`kernel/build.rs`): generates the kernel's platform
  table (RAM, devices, interrupt numbers, PSCI conduit) and link address.
  The kernel finds its interrupt controller, timer and early console by
  class and compatible string.
* **Boot compositions** grant devices by name; validation checks them
  against the FHD.
* **Driver binding** (`flare_cm::binding`) matches compatible strings to
  driver manifests.
* **`cargo xtask run`** reads `[emulation]` to start QEMU.

## Relationship to device tree

FHD is deliberately close to device tree in content (compatible strings,
`reg`, interrupts, status) so vendor DTS files can be converted, but it is
layered (SoC vs board), strict, and validated before anything boots. An
importer from DTS is on the roadmap.

## Porting checklist

1. If the SoC is new: write `hardware/soc/<vendor>/<soc>/platform.fhd`.
2. Write `hardware/boards/<vendor>/<board>/board.fhd`: SoC, memory, enabled
   devices, console.
3. Write or reuse driver components for the enabled devices.
4. Write a boot composition granting each driver its devices.
5. `cargo xtask fhd <vendor>/<board>`, then build and boot.
