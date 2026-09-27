# Src/Drivers/Touch — Touch Input Driver (Component J)

**Milestone:** M6. **HW map ref:** §9, §7 (INT/reset GPIO), §3 (IRQ).

## Contract
- **Inputs (all VERIFY):** touch bus (I2C vs SPI), controller IC + address, INT/reset
  GPIOs, coordinate range/scaling to 360×360.
- **Output:** WEC7 touch/stylus input events with panel-correct coordinates.
- **Reference:** AsteroidOS touch node/driver for IC identity and protocol (Rule 5).

**Deferred.** No code until §9 rows are CONFIRMED.
