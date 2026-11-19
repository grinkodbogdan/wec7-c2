# Flare kernel ABI (version 0)

The authoritative definition is the `flare-abi` crate
(`interfaces/abi/src/lib.rs`); this page explains it.

## Capabilities

A component addresses its authority by **slot** (0..31). Each slot holds at
most one capability: an object reference, a set of rights and a badge.

| Kind | Object | Rights used |
| --- | --- | --- |
| `endpoint` | IPC endpoint | `send`, `recv` |
| `mmio` | A device register window, mapped at boot | — (holding it is the authority) |
| `irq` | An interrupt line | — |
| `klog` | The kernel log | `write` (debug output, TTY input), `read` (become the log reader) |
| `dma` | Memory a driver may hand to its device, mapped into the driver | — (`DMA_PADDR` gives physical addresses) |
| `dma` (`shm:` grants) | Named shared memory: physically contiguous, the same pages for every holder, system lifetime | — |
| `input` | The Linux evdev device (`/dev/input/eventN`) a driver feeds | `write` |
| `serial` | The byte stream of a Linux character device (`/dev/<name>`) | `read`, `write` |

Rules:

* **No ambient authority.** A system call names a slot; an empty slot is
  `E_NO_CAP`, the wrong kind is `E_WRONG_TYPE`, a missing right is `E_RIGHTS`.
* **Derivation only removes rights.** `CAP_DERIVE` creates a child with a
  subset of the parent's rights. A badge can be set once, on an unbadged
  endpoint capability; servers see the sender's badge on every message.
* **Revocation follows provenance.** `CAP_REVOKE` deletes every capability
  derived from the given one, wherever it was delegated. `CAP_DELETE`
  removes one capability and re-parents its children.
* When a component dies, everything it held — and everything derived from
  that — is revoked; MMIO mappings and IRQ/log bindings go with it.

## System calls

`svc #0` with the number in `x8` and arguments in `x0..x7`. `x0` returns the
status (0 = success, else an error code).

| # | Name | Arguments | Returns |
| --- | --- | --- | --- |
| 0 | `DEBUG_WRITE` | klog slot, ptr, len | |
| 1 | `YIELD` | | |
| 2 | `SEND` | ep slot, label, w0..w5 | blocks until received |
| 3 | `RECV` | ep slot | `x1` badge, `x2` label, `x3..x8` words |
| 4 | `CALL` | ep slot, label, w0..w5 | like `RECV` (the reply) |
| 5 | `REPLY` | label, w0..w5 | |
| 6 | `CAP_DERIVE` | slot, rights, badge | `x1` new slot |
| 7 | `CAP_REVOKE` | slot | |
| 8 | `CAP_DELETE` | slot | |
| 9 | `IRQ_BIND` | irq slot, ep slot (recv), bit | |
| 10 | `IRQ_ACK` | irq slot | |
| 11 | `KLOG_BIND` | klog slot (read), ep slot (recv), bit | |
| 12 | `KLOG_READ` | klog slot, ptr, len | `x1` bytes read |
| 13 | `EXIT` | code | does not return |
| 14 | `TTY_INPUT` | klog slot (write), ptr, len | |
| 15 | `DMA_PADDR` | dma slot, offset | `x1` physical address |
| 16 | `INPUT_REPORT` | input slot, op (0 describe, 1 events), ptr, len | |
| 17 | `SERIAL_IO` | serial slot, op (0 push, 1 pull, 2 bind ep+bit), a, b | `x1` bytes moved |

Errors: `E_NO_CAP` 1, `E_WRONG_TYPE` 2, `E_RIGHTS` 3, `E_INVALID_ARG` 4,
`E_NO_MEMORY` 5, `E_NO_SLOT` 6, `E_PEER_DIED` 7, `E_BAD_ADDRESS` 8,
`E_NO_REPLY` 9, `E_UNKNOWN_SYSCALL` 10.

## IPC

Synchronous rendezvous over endpoints; messages are a badge, a label and six
words, passed in registers (no copying through memory).

* `SEND` blocks until a receiver takes the message; `CALL` additionally
  waits for the receiver's `REPLY`. A server has one pending reply at a
  time; receiving again without replying fails the caller with
  `E_NO_REPLY`.
* If a server dies, callers waiting for its reply get `E_PEER_DIED`; queued
  senders stay queued and are served by the restarted incarnation.
* **Notifications**: interrupts and kernel-log activity set bits on an
  endpoint. A `RECV` returns pending bits first, as label `LABEL_NOTIFY`
  (`u64::MAX`) with the bits in word 0. This lets a single-threaded driver
  wait for clients and hardware in one place.
* An interrupt line is masked after each delivery until the driver calls
  `IRQ_ACK`, so level-triggered devices work without kernel knowledge of
  the device.

## Boot info

Every component starts at its ELF entry with `x0` pointing at a read-only
`BootInfo` page: magic, ABI version, component name, incarnation (0 at first
start, +1 per restart) and a table of its capabilities — slot, kind, local
name from the boot composition, and for MMIO the mapped address and size.
Components look capabilities up **by name** (`bootinfo.find("console")`)
rather than assuming slot numbers.

## Address-space layout

| Range | Contents |
| --- | --- |
| `0x2_0000_0000..0x2_1000_0000` | Component image (linked by `libs/rt/user.ld`; W^X enforced at load) |
| `0x2_1000_0000..0x2_2000_0000` | MMIO capabilities, in grant order |
| `0x2_2000_0000` | `BootInfo` (read-only) |
| `0x2_3fff_8000..0x2_4000_0000` | Stack (32 KiB) |

Everything outside this 1 GiB region belongs to the kernel and faults if
touched from EL0.

## Service protocols

`flare.display@1.0` (`flare_abi::proto::display`): a framebuffer in shared
memory. The display driver owns a header at offset 0 (magic, state, size,
stride, format, generation, physical size) and scans out the pixels at
`fb_offset`. Linux sees it as `/dev/dri/card0` (KMS) and `/dev/fb0`.


`flare.console@1.0` (`flare_abi::proto::console`): requests are `CALL`s;
the label's low 32 bits are the operation, the high 32 bits a byte count;
bytes are packed little-endian into the six words (48 bytes per message).
`WRITE` (1) prints bytes — the server line-buffers per client badge so
concurrent clients never interleave within a line; `POLL_INPUT` (2) returns
buffered input, with the count in the reply label.
