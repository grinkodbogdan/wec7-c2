# Android Translation Layer on Flare

This directory builds [Android Translation Layer](https://gitlab.com/android_translation_layer/android_translation_layer)
(ATL) for Flare, with a native display backend in place of ATL's GTK one.
See [docs/android.md](../../../docs/android.md) for how it works.

* `build.sh OUT` fetches ATL at a pinned commit, applies `patches/`, and
  builds the framework, the JNI library and `atl-run`. It stages them as
  `/opt/atl`. `cargo xtask build` runs it when its inputs change.
* `flare/` is the native backend: view nodes, cairo/Pango drawing, bitmaps,
  text, the Arcan window and input, and the launcher.
* `gen_stubs.py` generates logging stubs for natives without a backend.
* `apps/views` is the test app that `cargo xtask test` runs in the phone
  shell.

## License

ATL is licensed under the GNU General Public License, version 3 or later.
The files in this directory are derived from ATL: the backend in `flare/`
ports ATL's JNI natives, and `patches/` modifies ATL. They are therefore
also GPL-3.0-or-later. They are built into a separate component, `/opt/atl`,
which the rest of Flare only starts as a program.
