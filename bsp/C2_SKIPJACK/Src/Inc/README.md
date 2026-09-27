# Src/Inc — Shared Headers (Register Definitions)

**HW map ref:** all sections. This folder will hold the C header(s) that encode the
hardware constants — bases, offsets, IRQ numbers, clock IDs — that the OAL and drivers
consume.

## Rule: headers mirror the hardware map

Every constant defined here **must** correspond to a row in
`docs/01-hardware-map.md`, and its comment must cite the section and confidence:

```c
/* GIC-400 distributor. HW map §3. Confidence: FAMILY (VERIFY vs skipjack dts). */
#define MSM8909_GICD_BASE   0x0B000000u
```

A constant may not appear here at higher confidence than its HW map row. When a value is
still `VERIFY`, either omit it or guard it so it cannot compile into a hardware path
(Rule 8, Rule 9).

Planned header(s): `msm8909_regs.h` (SoC register map), `c2_platform.h` (board-level:
pins, partitions, memory layout) — created as their HW map rows reach `CONFIRMED`.
