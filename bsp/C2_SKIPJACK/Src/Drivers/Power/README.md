# Src/Drivers/Power — Power / Battery (Component L)

**Milestone:** M8. **HW map ref:** §12.

## Contract
- **Inputs (all VERIFY):** PM8909 via SPMI (base + sid), fuel gauge path, charger IC,
  hardware watchdog base/timeout, suspend/resume states.
- **Output:** battery/charge reporting, watchdog service, suspend/resume.
- **Note:** basic watchdog handling is needed *earlier* (see OAL `init/`, HW map §12) so
  first boot doesn't reset; full power management is the M8 scope.

**Deferred.** No code until §12 rows are CONFIRMED.
