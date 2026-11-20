# linuxdrv: Linux's drivers in a Flare process

The Linux driver host ([docs/linux-drivers.md](../../../docs/linux-drivers.md)).

* `build.sh OUT SYSROOT` fetches LKL at a pinned commit, applies `lkl/`'s
  patches once, configures it and builds `OUT/linuxdrv`. `cargo xtask build`
  runs it when its inputs change and the composition gives Linux hardware.
* `linuxdrv.c`: LKL's host operations over `/dev/flarehw`, and the report
  of what Linux found.
* `lkl/qrt.patch`, `lkl/config-arm64`: from QRT (github.com/grinkodbogdan/qrt),
  the configuration trimmed for Flare. `lkl/flare.patch`: interrupts and
  SMC calls through the host. `lkl/config-flare`: Flare's options.

## License

Linux and LKL are licensed under GPL-2.0, and so are the patches and
`linuxdrv.c`, which is linked with LKL. The program is a separate
component, `/usr/bin/linuxdrv`, which the rest of Flare only runs.
