"""A minimal flattened device tree (DTB) writer."""
import struct

class Fdt:
    def __init__(self):
        self.struct = bytearray()
        self.strings = bytearray()
        self.offsets = {}

    def _name(self, n):
        if n not in self.offsets:
            self.offsets[n] = len(self.strings)
            self.strings += n.encode() + b"\0"
        return self.offsets[n]

    def _pad(self):
        while len(self.struct) % 4:
            self.struct.append(0)

    def begin(self, name):
        self.struct += struct.pack(">I", 1) + name.encode() + b"\0"
        self._pad()

    def end(self):
        self.struct += struct.pack(">I", 2)

    def prop(self, name, value):
        if isinstance(value, str):
            data = value.encode() + b"\0"
        elif isinstance(value, list) and value and isinstance(value[0], str):
            data = b"".join(v.encode() + b"\0" for v in value)
        elif isinstance(value, list):
            data = b"".join(struct.pack(">I", v) for v in value)
        else:
            data = bytes(value)
        self.struct += struct.pack(">III", 3, len(data), self._name(name)) + data
        self._pad()

    def blob(self):
        self.struct += struct.pack(">I", 9)
        rsv = b"\0" * 16
        off_rsv = 40
        off_struct = off_rsv + len(rsv)
        off_strings = off_struct + len(self.struct)
        total = off_strings + len(self.strings)
        hdr = struct.pack(">10I", 0xD00DFEED, total, off_struct, off_strings, off_rsv, 17, 16, 0,
                          len(self.strings), len(self.struct))
        return hdr + rsv + bytes(self.struct) + bytes(self.strings)


