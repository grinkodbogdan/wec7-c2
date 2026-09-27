# Src/Drivers/Display — Display Driver (Component I)

**Milestone:** M5 (after M4 boot proven — Rule 10). **HW map ref:** §8.

## Contract
- **Inputs (all VERIFY):** MDSS/MDP base, MIPI DSI config, panel init command sequence,
  pixel format, framebuffer carveout (HW map §2, §8), display reset/TE GPIOs (§7).
- **Output:** a linear framebuffer WEC7 GDI can paint; 360×360 updates to the round panel.
- **Reference:** AsteroidOS panel driver is a *reference* for the DSI init sequence only
  (Rule 5) — re-implemented, not linked.

**Deferred.** No code until M4 is DONE and §8 rows are CONFIRMED.
