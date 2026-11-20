#!/usr/bin/env python3
"""A device tree for Linux's drivers in the driver host (compat/linux/linuxdrv),
from the board's FHD: the interrupt controller and the devices the composition
gives Linux ([linux] hardware), with their compatible strings, registers and
interrupts.

    fhd2dtb.py SPEC.json OUT.dtb

SPEC (written by `cargo xtask build`): {"model", "intc": {"compatible", "regs"},
"devices": [{"name", "compatible", "regs", "irqs"}]}; interrupts are GIC
INTIDs. AMBA (PrimeCell) devices get the fixed APB clock their bus needs.
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fdt import Fdt  # noqa: E402

spec = json.load(open(sys.argv[1]))
GIC, APB = 1, 2
t = Fdt()
t.begin("")
t.prop("#address-cells", [2])
t.prop("#size-cells", [2])
t.prop("model", spec.get("model", "Flare board"))
t.prop("compatible", ["flare,linux-driver-host"])
t.prop("interrupt-parent", [GIC])
ic = spec["intc"]
t.begin("interrupt-controller@%x" % ic["regs"][0][0])
t.prop("compatible", ic["compatible"])
t.prop("interrupt-controller", b"")
t.prop("#interrupt-cells", [3])
t.prop("reg", [c for b, s in ic["regs"] for c in (b >> 32, b & 0xffffffff, s >> 32, s & 0xffffffff)])
t.prop("phandle", [GIC])
t.end()
t.begin("apb-pclk")
t.prop("compatible", "fixed-clock")
t.prop("#clock-cells", [0])
t.prop("clock-frequency", [24000000])
t.prop("clock-output-names", "clk24mhz")
t.prop("phandle", [APB])
t.end()
for d in spec["devices"]:
    t.begin("%s@%x" % (d["name"], d["regs"][0][0]) if d["regs"] else d["name"])
    t.prop("compatible", d["compatible"])
    if d["regs"]:
        t.prop("reg", [c for b, s in d["regs"] for c in (b >> 32, b & 0xffffffff, s >> 32, s & 0xffffffff)])
    if d["irqs"]:
        cells = []
        for i in d["irqs"]:
            # <type number flags>: SPI = 0 (INTID - 32), PPI = 1 (INTID - 16); level high
            cells += [0, i - 32, 4] if i >= 32 else [1, i - 16, 4]
        t.prop("interrupts", cells)
    if "arm,primecell" in d["compatible"]:
        t.prop("clocks", [APB])
        t.prop("clock-names", "apb_pclk")
    t.end()
t.end()
open(sys.argv[2], "wb").write(t.blob())
