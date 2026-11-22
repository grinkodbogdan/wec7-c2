#!/usr/bin/env python3
"""Look at and trim flattened device trees for the Linux driver host.

    dtb.py info BOARD.dtb [--all]
        List the enabled nodes that have registers or interrupts.

    dtb.py fhd BOARD.dtb NODE NAME
        Print the FHD entry for a device of Linux's tree (NODE: its path, or
        a unique part of it), to paste into the SoC's platform.fhd: its
        compatible strings, register windows and GIC interrupts (as INTIDs).
        Giving a device to Linux is then: this entry, `NAME` in the
        composition's [linux] hardware, the node not dropped in linux.txt,
        and its driver in the driver host's kernel config.

    dtb.py grants BOARD.dtb
        What Linux uses in a (trimmed) tree, for the kernel to grant it
        (`[linux] devicetree = true`): one line per register window,
        "w BASE SIZE dev|mem PATH" ("mem" for reserved memory: RAM Linux
        maps itself), and per GIC interrupt, "i INTID PATH". The interrupt
        controller, the CPUs and RAM are left out; so are interrupts that
        go to another controller (Linux's own: TLMM, SPMI...).

    dtb.py hidden BOARD.dtb PATH...
        The compatible strings `trim --hide PATH...` renames ("qcom,x" is
        kept as "flr-,x" until the driver host restores it): one line.

    dtb.py trim IN.dtb OUT.dtb --allow BASE+SIZE... [--drop PATH...] [--delprop NAME...]
               [--setprop PATH:NAME=STRING...]
               [--hide PATH...]
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
        out.append((translate(par, a), s))
    return out


def translate(bus, a):
    """A child address of `bus` as a CPU address, through each bus's
    'ranges' (empty: the same addresses)."""
    while bus is not None and bus["parent"] is not None:
        r = bus["props"].get("ranges", b"")
        if r:
            ca = num(bus, "#address-cells", 2)
            pa = num(bus["parent"], "#address-cells", 2)
            sz = num(bus, "#size-cells", 1)
            c, step = cells(r), ca + pa + sz
            for i in range(0, len(c) - step + 1, step):
                def val(xs):
                    v = 0
                    for x in xs:
                        v = (v << 32) | x
                    return v
                cb, pb, ln = val(c[i:i + ca]), val(c[i + ca:i + ca + pa]), val(c[i + ca + pa:i + step])
                if cb <= a < cb + ln:
                    a = a - cb + pb
                    break
        bus = bus["parent"]
    return a


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


def fhd(path, which, name):
    root = parse(open(path, "rb").read())
    nodes = ([n for n in walk(root) if n["path"] == which]
             or [n for n in walk(root) if n["path"].rsplit("/", 1)[-1] == which]
             or [n for n in walk(root) if which in n["path"].rsplit("/", 1)[-1]])
    if len(nodes) != 1:
        sys.exit("dtb.py: %d nodes match '%s'%s" % (len(nodes), which,
                 "".join("\n  " + n["path"] for n in nodes[:20])))
    n = nodes[0]
    pr = n["props"]
    # The GIC, by phandle: only its interrupts are Flare's to grant (others
    # belong to an interrupt controller Linux itself drives: TLMM, SPMI...).
    gics = {num(x, "phandle", None) for x in walk(root)
            if "interrupt-controller" in x["props"] and set(strs(x["props"].get("compatible", b""))) & GIC}
    up, parent = n, None
    while up is not None and parent is None:
        if "interrupt-parent" in up["props"]:
            parent = cells(up["props"]["interrupt-parent"])[0]
        up = up["parent"]
    irqs, notes = [], []
    if "interrupts" in pr:
        c = cells(pr["interrupts"])
        if parent in gics:
            for i in range(0, len(c) - 2, 3):
                kind, nr = c[i], c[i + 1]
                irqs.append(nr + (32 if kind == 0 else 16))
        else:
            notes.append("its interrupts go to another controller (Linux's): none to grant")
    if "interrupts-extended" in pr:
        notes.append("interrupts-extended: check by hand")
    print("[device.%s]" % name)
    if "compatible" in pr:
        print("compatible = [%s]" % ", ".join('"%s"' % x for x in strs(pr["compatible"])))
    rs = regs(n)
    if len(rs) == 1:
        print("reg = [%#010x, %#x]" % rs[0])
    elif rs:
        print("reg = [%s]" % ", ".join("[%#010x, %#x]" % r for r in rs))
    if irqs:
        print("interrupts = [%s]    # from %s" % (", ".join(map(str, irqs)), n["path"]))
    print('status = "disabled"    # the board enables it')
    for x in notes:
        print("# " + x)


def gic_phandles(root):
    return {num(x, "phandle", None) for x in walk(root)
            if "interrupt-controller" in x["props"] and set(strs(x["props"].get("compatible", b""))) & GIC}


def irq_parent(n):
    while n is not None:
        if "interrupt-parent" in n["props"]:
            return cells(n["props"]["interrupt-parent"])[0]
        n = n["parent"]
    return None


def gic_irqs(n, gics):
    """INTIDs of a node's SPIs at the GIC (PPIs are the CPUs': Flare's)."""
    pr, out = n["props"], []
    if "interrupts" in pr and irq_parent(n) in gics:
        c = cells(pr["interrupts"])
        out += [c[i + 1] + 32 for i in range(0, len(c) - 2, 3) if c[i] == 0]
    if "interrupts-extended" in pr:
        c, i = cells(pr["interrupts-extended"]), 0
        while i < len(c):
            ph = c[i]
            if ph in gics:
                if c[i + 1] == 0:
                    out.append(c[i + 2] + 32)
                i += 4
            else:
                # another controller: its #interrupt-cells
                ctl = next((x for x in walk(n_root(n)) if num(x, "phandle", None) == ph), None)
                i += 1 + num(ctl, "#interrupt-cells", 2)
    return out


def n_root(n):
    while n["parent"] is not None:
        n = n["parent"]
    return n


def grants(path):
    root = parse(open(path, "rb").read())
    gics = gic_phandles(root)
    for n in walk(root):
        p = n["path"]
        if not enabled(n) or p == "/" or p.startswith(("/cpus", "/memory", "/chosen", "/aliases")):
            continue
        pr = n["props"]
        if set(strs(pr.get("compatible", b""))) & GIC or strs(pr.get("device_type", b"x")) == ["memory"]:
            continue
        kind = "mem" if p.startswith("/reserved-memory/") else "dev"
        for a, sz in regs(n):
            if sz:
                print("w %#x %#x %s %s" % (a, sz, kind, p))
        for i in gic_irqs(n, gics):
            print("i %d %s" % (i, p))


def hidden(path, paths):
    root = parse(open(path, "rb").read())
    out = []
    for n in walk(root):
        if any(n["path"] == h or n["path"].startswith(h.rstrip("/") + "/") for h in paths):
            out += [c for c in strs(n["props"].get("compatible", b"")) if c.startswith("qcom,") and c not in out]
    print(" ".join(out))


def emit(f, n):
    f.begin("" if n["parent"] is None else n["name"])
    for k, v in n["props"].items():
        f.prop(k, v)
    for c in n["children"]:
        emit(f, c)
    f.end()


def trim(src, dst, allow, drop, delprops=(), setprops=(), hide=()):
    root = parse(open(src, "rb").read())
    off = []
    for n in walk(root):
        pr = n["props"]
        # Hidden until the driver host gives it back (firmware first): its
        # "qcom," compatibles become "flr-," in place - the same length, so
        # the host can restore them in the tree Linux reads (`unhide`).
        if any(n["path"] == h or n["path"].startswith(h.rstrip("/") + "/") for h in hide) and "compatible" in pr:
            pr["compatible"] = b"\0".join(
                (b"flr-," + c[5:]) if c.startswith(b"qcom,") else c for c in pr["compatible"].split(b"\0"))
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
    elif len(a) >= 3 and a[0] == "hidden":
        hidden(a[1], a[2:])
    elif len(a) == 2 and a[0] == "grants":
        grants(a[1])
    elif len(a) == 4 and a[0] == "fhd":
        fhd(a[1], a[2], a[3])
    elif len(a) >= 3 and a[0] == "trim":
        allow, drop, delprops, setprops, hide, mode = [], [], [], [], [], None
        for x in a[3:]:
            if x in ("--allow", "--drop", "--delprop", "--setprop", "--hide"):
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
            elif mode == "--hide":
                hide.append(x)
            else:
                sys.exit("dtb.py: unexpected '%s'" % x)
        trim(a[1], a[2], allow, drop, delprops, setprops, hide)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
