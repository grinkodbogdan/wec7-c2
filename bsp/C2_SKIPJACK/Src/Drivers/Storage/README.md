# Src/Drivers/Storage — Storage Stack (Component K)

**Milestone:** M7. **HW map ref:** §10, §11 (partitions).

## Contract
- **Inputs (all VERIFY):** SDHCI/SDCC controller base, eMMC config, GPT partition layout.
- **Output:** block access + persistent filesystem for WEC7.
- **Safety:** driver must respect the partition map from §11 and never write firmware
  partitions (`modem`, `tz`, bootloader). Backups (M0) precede any write testing.

**Deferred.** No code until §10/§11 rows are CONFIRMED.
