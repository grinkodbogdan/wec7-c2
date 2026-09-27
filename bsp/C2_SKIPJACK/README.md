# C2_SKIPJACK — WEC7 BSP (skeleton)

C2-specific WEC7 BSP for the Mobvoi TicWatch C2 (Snapdragon Wear 2100 / MSM8909W).

**State:** M3 skeleton — folder contracts only, no hardware code. See `../README.md` for
the component ↔ milestone map, and `../../docs/01-hardware-map.md` for the binding
hardware spec every module must cite.

```
C2_SKIPJACK/
├── Src/
│   ├── Startup/     ARM startup assembly, CPU init, kernel entry (component A).
│   ├── Bootloader/  Bootloader handoff analysis & shim (component A / HW map §13.1).
│   ├── Oal/         OEM Adaptation Layer: memory, IRQ, timers, clocks, GPIO, early debug (B–H).
│   ├── Kernel/      Kernel config glue.
│   ├── Drivers/
│   │   ├── Display/ Component I (M5).
│   │   ├── Touch/   Component J (M6).
│   │   ├── Storage/ Component K (M7).
│   │   └── Power/   Component L (M8).
│   └── Inc/         Shared register/definition headers (sourced from HW map).
└── Files/           Image build config (platform.bib/.reg/.dat/.db) — TBD at M2/M3.
```

To be created at M2: `BUILD.md` (reproducibility manifest, per `docs/03-toolchain-setup.md`).
