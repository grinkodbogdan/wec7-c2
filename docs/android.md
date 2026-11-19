# Android apps without Android

Flare runs Android apps **straight from their APKs** with `droid`, a small
runtime in `compat/android/droid`. It does not boot Android userspace: no
`init`, `servicemanager`, zygote, ART or `system_server`. It works like
Vinix's or a unikernel's approach to compatibility: give apps the surface
they call, and nothing else.

```
rc: for apk in /data/app/*.apk; do droid $apk; done
droid: org.flare.kotlin: launching org.flare.kotlin.MainActivity (32 classes, 1 dex)
I/FlareKotlin: Kotlin onCreate on Flare (API 35)
I/FlareKotlin: data class: Task(id=1, title=boot kernel, done=true), copy: Task(id=1, title=boot kernel, done=false), equal: true
I/FlareKotlin: groupingBy: {a=3, b=2, c=1}
I/FlareKotlin: onResume: kotlin app is running
droid: org.flare.kotlin.MainActivity finished cleanly (0 missing APIs)
```

`droid` is one static Linux arm64 executable (about 1 MB). It runs under
Flare's Linux personality like any other program. Starting an app takes
tens of milliseconds, and the only memory used is what the app itself uses.

## How it works

| Stage | Module | What it does |
| --- | --- | --- |
| APK | `zip.rs` | Reads the APK's zip directory: stored and deflated entries |
| Manifest | `axml.rs` | Decodes Android's binary XML and finds the package and its `MAIN`/`LAUNCHER` activity |
| Bytecode | `dex.rs` | Parses the DEX format: strings, types, protos, fields, methods, class data, code items with try/catch tables, encoded static values (`classes*.dex`, multidex) |
| Execution | `vm.rs` | Interprets Dalvik bytecode. Classes load lazily and initialise with `<clinit>`. It resolves virtual, super, direct, static and interface calls (including default methods), throws exceptions through the try tables, checks casts and array bounds, and handles every arithmetic, conversion, switch and array opcode the compilers emit |
| Libraries | `natives.rs` | The Java core library and the Android API, implemented natively in Rust (see below) |
| Lifecycle | `main.rs` | Instantiates the activity and runs `onCreate` → `onStart` → `onPostCreate` → `onResume` → `onPause` → `onStop` → `onDestroy`, as ActivityThread would. `finish()` is honoured |

App classes can extend native classes (an `Activity`, an `Exception`).
Native code calls back into app code for overridden `toString`, `equals`,
`hashCode`, `compareTo`, `Runnable.run`, `Comparator.compare` and
`Consumer.accept`.

### Native class library

* `java.lang`:
  * `Object`, `Class`, `String` (UTF-16 semantics), `StringBuilder` and `StringBuffer`;
  * boxed types and their parsers, `Math`, `System` (`out`/`err`, `arraycopy`, clocks), `Thread` (run to completion), `Enum`, `Objects`;
  * `Throwable` and about 35 exception classes;
  * `String.format` / `printf` with flags, width and precision.
* `java.util`:
  * `ArrayList`, `LinkedList`, `HashMap` and `LinkedHashMap` (entries write through), `HashSet` and `LinkedHashSet`, iterators with `remove`;
  * `Arrays` (`asList`, `sort`, `copyOf`, `toString`, `fill`), `Collections`, `Locale`.
* `java.lang.reflect.Array.newInstance`, used for multi-dimensional arrays.
* `android.*`:
  * `Activity`, with `Context`, `ContextWrapper` and `ContextThemeWrapper` beneath it;
  * `Bundle`, `Log` (printed as `I/TAG: message`), `Build` and `Build.VERSION` (API 35), `SystemClock`.

The Kotlin standard library is not reimplemented. Like on Android, it is
part of the app's own DEX, and it runs on top of the Java core above.

### When something is missing

A class, method, field or opcode that droid does not provide raises the
Java exception a real VM would: `NoClassDefFoundError`,
`NoSuchMethodError` and so on. The app can catch it. droid also prints it
once, for example:

```
droid: missing java.util.TreeMap (not in the app and not provided by droid)
```

The boot test fails on any `droid: missing` line. That makes the next API
to add obvious: running a new app shows exactly what it needs.

## Test apps

The test apps live in `compat/android/apps`. `cargo xtask test` builds them
from source and runs them on Flare.

| App | Language | Exercises |
| --- | --- | --- |
| `hello` | Java | Lifecycle, interfaces, recursion, collections, `String.format`, exceptions, lambdas, `Build` |
| `kotlin` | Kotlin | Data classes (`copy`, `equals`, `toString`), sealed classes with `when`, extension functions, collection operators (`filter`/`map`/`groupingBy`/`sumOf`), ranges, null safety, `runCatching` |
| `selftest` | Java | 51 checks: integer, long and floating-point edge cases, packed, sparse and string switches, static initialisers, 2-D arrays, enums, default methods, abstract classes, custom `equals` as map keys, checked exceptions, `finally`, `ClassCastException`, NPE, array bounds, formatting, sorting with `Comparable` and lambdas, `LinkedHashMap` order, iterator removal, `arraycopy`, `synchronized`, `Bundle` |

The build is a normal Android toolchain:

* `javac` and `kotlinc` 2.1 compile the sources.
* Java apps are dexed with `d8`. Kotlin apps are shrunk and optimised with `R8` release mode, like a store build.
* `aapt2` links them against `android.jar` (API 35).

xtask downloads build-tools 35.0.1, platform 35 and the Kotlin compiler
once, pinned by SHA-256. A host JDK is the only extra requirement. APKs
are rebuilt only when their sources change. They are installed as
`/data/app/*.apk`, with `droid` in `/usr/bin`.

Run any APK yourself, on Flare or on a Linux host:

```sh
cargo run -p droid -- path/to/app.apk
```

## Limits and next steps

* **No display or input yet.** After `onResume` the activity is paused and
  stopped. `setContentView` is accepted and reported as missing. Next: a
  virtio-gpu display service and input on Flare, then `View`, `Canvas` and
  a message loop (`Looper`/`Handler`), so apps can draw and react.
* **Single-threaded.** `Thread.start` runs the thread to completion.
* **No resources yet.** `resources.arsc`, `R.string` and layouts still
  need to be wired up.
* **No JNI.** An app's own native libraries (`lib/arm64-v8a/*.so`) are
  not loaded. Calls raise `UnsatisfiedLinkError`. The Linux personality
  can already load shared libraries, so this can be added.
* **No reflection** beyond `Class` names, and no `invoke-custom`
  (`d8 --min-api 24` desugars lambdas, so apps do not need it).
* The interpreter has no JIT, and the garbage collector is reference
  counting, so object cycles leak. That is fine for app-sized runs and can
  be improved later.
