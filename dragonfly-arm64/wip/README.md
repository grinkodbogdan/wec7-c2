# Work in progress

Patches that are not finished and not applied by `build.sh`.

- `smp-secondary-cpus.patch`: the start of multi-CPU support: the entry
  point for secondary CPUs, per-CPU interrupt controller setup, and
  inter-processor interrupts. Still missing: the code that asks the
  firmware to start each CPU (PSCI `CPU_ON`), the per-CPU startup and
  hand-shake with the boot CPU, and any testing. It was written against
  an earlier commit and needs rebasing.
