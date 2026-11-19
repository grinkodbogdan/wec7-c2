# fdb: the Flare Debug Bridge

`fdb` is Flare's equivalent of Android's `adb`. From the development
machine it gives you a shell on the device, file transfer, app install and
launch, supervisor control, screenshots, and a fast loop for UI work: push
an edited Arcan appl and see it running a few seconds later, with no
rebuild and no reboot.

```sh
cargo xtask run                       # boot Flare in QEMU (the bridge is target/fdb.sock)
cargo fdb devices                     # flare device (fdbd 0.1.0), up 12.58 s ...
cargo fdb shell 'uname -a; ls /dev'   # output streams back; the exit status is fdb's
cargo fdb shell                       # interactive shell
cargo fdb push build/thing /tmp/      # files or whole directories
cargo fdb pull /tmp/arcan.sqlite .
cargo fdb run target/apks/kotlin.apk  # install to /data/app and run it with droid
cargo fdb appl compat/arcan/appl/flarehome   # push the appl, restart the UI with it
cargo fdb log                         # follow the UI's output
cargo fdb ctl restart storage         # ask the supervisor
cargo fdb ctl trace arcan             # trace a program's system calls to the console (trace off)
cargo fdb screenshot shot.png         # the display, as PNG
```

`cargo fdb` is an alias for `cargo run -p fdb --`. The `fdb` binary works
on its own too, for example after `cargo install --path tools/fdb`.

Measured against the QEMU board:

| Operation | Time |
| --- | --- |
| shell round trip | 0.15 s |
| 293 KiB push | 0.1 s |
| install and run an APK | 0.19 s |
| screenshot | 0.4 s |
| appl push and restart | 0.26 s |

After an appl restart, the shell is drawing again within a few seconds.

## How it works

```
 host: fdb ──── unix socket target/fdb.sock (QEMU chardev) ────┐
                                                               │ virtio console
 Flare: virtio-console driver (user space) ── serial:fdb capability
        Linux personality: /dev/fdb ── fdbd (runs commands, files, screenshots)
```

* **Transport.** The board's virtio console carries the bridge. On a phone,
  the same byte stream would run over a USB gadget function or a network
  socket. `FDB_SOCKET` selects the endpoint: a Unix socket path, or
  `tcp:host:port`. It defaults to `target/fdb.sock`, which `cargo xtask`
  gives QEMU.
* **Serial capabilities.** A `serial:<name>` grant connects a driver to the
  Linux character device `/dev/<name>`, through the `SERIAL_IO` system
  call:
  * the driver pushes received bytes to Linux;
  * it pulls what Linux wrote;
  * it binds a notification for when Linux writes.

  Each direction buffers up to 1 MiB; writers see `EAGAIN` when it is full.
* **The driver** (`drivers/virtio/console`) owns the virtio-console
  transport: a receive queue that feeds `/dev/fdb`, and a transmit queue
  that drains it. Like every Flare driver it can be restarted while in use.
* **fdbd** (`compat/linux/fdbd`) is an ordinary Rust program on the Linux
  personality, started by `/etc/rc`:
  * it runs commands under `/bin/sh` and streams their stdout and stderr;
  * it reads and writes files and creates directories;
  * it captures screenshots from `/dev/fb0`, a new fbdev view of the
    display shared memory.
* **Protocol** (`tools/fdb/src/lib.rs`): frames of `0xFD kind len payload`.
  The magic byte lets either side resynchronise after a peer disappeared
  mid-frame. The `fdb` crate is both the host CLI and a library;
  `cargo xtask test` uses the library to check the bridge on every boot.

## UI development loop

`/usr/bin/flare-ui [appl]` (re)starts Arcan with an appl from
`/usr/share/arcan/appl`. The shell's output goes to the console and to
`/tmp/flare-ui.log`. `fdb appl <dir> [name]` pushes the directory and calls
it. So working on the phone shell is:

```sh
$EDITOR compat/arcan/appl/flarehome/flarehome.lua
cargo fdb appl compat/arcan/appl/flarehome && cargo fdb screenshot now.png
```

Launcher icons are drawn by `compat/arcan/appl/flarehome/icons/make_icons.py`;
rerun it after changing the art.
