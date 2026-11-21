#!/usr/bin/env python3
"""Look at and trim flattened device trees for the Linux driver host.

    dtb.py info BOARD.dtb [--all]
        List the enabled nodes that have registers or interrupts.

    dtb.py trim IN.dtb OUT.dtb --allow BASE+SIZE... [--drop PATH...] [--delprop NAME...]
               [--setprop PATH:NAME=STRING...]
        Disable every device whose registers are not all inside an allowed
        window, and the nodes named by --drop (with what is under them). The
        interrupt controller is kept. --delprop removes a property from every
        node (references to providers Linux does not have). Linux then only probes the devices the
        composition gives it: a board's whole mainline device tree can be
        used without Linux touching hardware that Flare kept.

Addresses are taken as CPU physical addresses: buses are assumed to map 1:1
(true of the SoCs Flare is ported to; checked against 'ranges').
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fdt import Fdt  # noqa: E402

GIC = {"arm,gic-v2", "arm,gic-400", "arm,cortex-a15-gic", "arm,cortex-a7-gic", "qcom,msm-qgic2", "arm,gic-v3"}


def parse(blob):
    magic, _, off_struct, off_strings = struct.unpack(">IIII", blob[:16])
    if magic != 0xD00DFEED:
        sys.exit("not a device tree")

    def string(o):
        e = blob.index(b"\0", off_strings + o)
        return blob[off_strings + o:e].decode()

    p = off_struct
    stack, root = [], None
    while True:
        (tok,) = struct.unpack(">I", blob[p:p + 4])
        p += 4
        if tok == 1:
            e = blob.index(b"\0", p)
            name = blob[p:e].decode()
            p = (e + 4) & ~3
            parent = stack[-1] if stack else None
            node = {"name": name, "props": {}, "children": [], "parent": parent}
            node["path"] = "/" if parent is None else parent["path"].rstrip("/") + "/" + name
            if parent:
                parent["children"].append(node)
            else:
                root = node
            stack.append(node)
        elif tok == 2:
            stack.pop()
        elif tok == 3:
            ln, no = struct.unpack(">II", blob[p:p + 8])
            p += 8
            stack[-1]["props"][string(no)] = blob[p:p + ln]
            p = (p + ln + 3) & ~3
        elif tok == 9:
            return root


def walk(n):
    yield n
    for c in n["children"]:
        yield from walk(c)


def cells(v):
    return list(struct.unpack(">%dI" % (len(v) // 4), v[: len(v) // 4 * 4]))


def strs(v):
    return [s.decode() for s in v.split(b"\0") if s]


def num(n, key, default):
    return cells(n["props"][key])[0] if n and key in n["props"] else default


def regs(n):
    """(address, size) pairs, or [] where 'reg' is not a CPU address (a node
    on an I2C, SPMI or other non-memory-mapped bus)."""
    par = n["parent"]
    if "reg" not in n["props"] or par is None:
        return []
    up = par
    while up is not None and up["parent"] is not None:
        # Memory-mapped only through a chain of buses that have 'ranges'.
        if "ranges" not in up["props"]:
            return []
        up = up["parent"]
    ac, sc = num(par, "#address-cells", 2), num(par, "#size-cells", 1)
    c = cells(n["props"]["reg"])
    out = []
    for i in range(0, len(c) - (ac + sc) + 1, ac + sc):
        a = s = 0
        for x in c[i:i + ac]:
            a = (a << 32) | x
        for x in c[i + ac:i + ac + sc]:
            s = (s << 32) | x
        out.append((a, s))
    return out


def status(n):
    return strs(n["props"]["status"])[0] if "status" in n["props"] else "okay"


def enabled(n):
    while n is not None:
        if status(n) not in ("okay", "ok"):
            return False
        n = n["parent"]
    return True


def info(path, everything):
    for n in walk(parse(open(path, "rb").read())):
        pr = n["props"]
        if not everything and not enabled(n):
            continue
        if "reg" not in pr and "interrupts" not in pr:
            continue
        comp = ",".join(strs(pr["compatible"])) if "compatible" in pr else ""
        rs = " ".join("%#x+%#x" % r for r in regs(n))
        irq = " ".join(map(str, cells(pr["interrupts"]))) if "interrupts" in pr else ""
        print("%-60s %-40s %s%s%s" % (n["path"], comp, rs, "  irq " + irq if irq else "",
                                      "" if enabled(n) else "  [disabled]"))


def emit(f, n):
    f.begin("" if n["parent"] is None else n["name"])
    for k, v in n["props"].items():
        f.prop(k, v)
    for c in n["children"]:
        emit(f, c)
    f.end()


def trim(src, dst, allow, drop, delprops=(), setprops=()):
    root = parse(open(src, "rb").read())
    off = []
    for n in walk(root):
        pr = n["props"]
        for p in delprops:
            pr.pop(p, None)
        for path, name, value in setprops:
            if n["path"] == path:
                pr[name] = value.encode() + b"\0"
        if n["parent"] is None:
            continue
        # Every node under a dropped path, not just the top one: Linux
        # creates some nodes' children whatever the parent's status (those
        # of /firmware).
        dropped = any(n["path"] == d or n["path"].startswith(d.rstrip("/") + "/") for d in drop)
        if not dropped and not enabled(n):
            continue
        if not dropped:
            if "compatible" in pr and GIC & set(strs(pr["compatible"])):
                continue
            rs = regs(n)
            if not rs or all(any(b <= a and a + s <= b + z for b, z in allow) for a, s in rs if s):
                continue
        pr["status"] = b"disabled\0"
        off.append(n["path"])
    f = Fdt()
    emit(f, root)
    open(dst, "wb").write(f.blob())
    for p in off:
        print("dtb: disabled %s" % p)


def window(s):
    a, _, z = s.partition("+")
    return int(a, 0), int(z, 0)


def main():
    a = sys.argv[1:]
    if len(a) >= 2 and a[0] == "info":
        info(a[1], "--all" in a)
    elif len(a) >= 3 and a[0] == "trim":
        allow, drop, delprops, setprops, mode = [], [], [], [], None
        for x in a[3:]:
            if x in ("--allow", "--drop", "--delprop", "--setprop"):
                mode = x
            elif mode == "--setprop":
                path, _, rest = x.partition(":")
                name, _, value = rest.partition("=")
                setprops.append((path, name, value))
            elif mode == "--allow":
                allow.append(window(x))
            elif mode == "--drop":
                drop.append(x)
            elif mode == "--delprop":
                delprops.append(x)
            else:
                sys.exit("dtb.py: unexpected '%s'" % x)
        trim(a[1], a[2], allow, drop, delprops, setprops)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
