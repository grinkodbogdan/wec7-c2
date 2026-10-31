# 03 — Debugging: screen, GPIO "blink", UART

On a watch with no serial header and no notification LED, you need signal from the
*earliest* code that runs. Three channels, from easiest to most reliable. Bring up at
least one before you touch anything else (project rule 10).

Reference code: [`../debug/fb_debug.c`](../debug/fb_debug.c),
[`../debug/post_codes.h`](../debug/post_codes.h),
[`../debug/gpio_blink.c`](../debug/gpio_blink.c).

---

## A. Screen debug (framebuffer paint) — do this first

The bootloader (ABL continuous-splash) lights the panel and leaves a linear framebuffer
in RAM. If you know its base + geometry (doc 01), you can **write pixels directly** long
before any display driver exists. Three escalating techniques:

1. **Full-screen color fill as a POST code.** Reached SEC? paint blue. Reached PEI?
   green. Hit the memory-map code? yellow. A faulting stage leaves the last color on
   screen — instant "how far did I get". This needs *nothing* but a pointer and a loop.
2. **Progress bar / quadrant blocks.** Divide the round 360×360 into regions; light one
   per milestone. Survives even with no font.
3. **Bitmap font text.** Once fills work, blit an 8×8 font to print hex values
   (addresses, register reads). `fb_debug.c` includes a minimal 8×8 printer.

Watch-specific notes:
- Panel is **round 360×360**, but the framebuffer is a normal rectangular buffer; corners
  are simply not visible. Keep debug output near center.
- Pixel format is typically 32bpp BGRA/ARGB — confirm from `/sys/class/graphics/fb0/*`
  and the DTS. Wrong byte order just means your colors are swapped, still useful.
- If the bootloader already turned the panel off by the time UEFI runs, you may need to
  re-poke the PMIC backlight regulator / a GPIO before fills show — fall back to GPIO
  blink (B) until then.

## B. GPIO "blink" debug — the LED substitute

The C2 has no classic notification LED, so "blinking LEDs" becomes **toggling a GPIO you
can perceive**:

- **Vibration motor** — strongest signal; blip it N times for POST code N. Great for
  "did we even reach `_ModuleEntryPoint`".
- **Display backlight GPIO/regulator** — flash the screen brightness.
- **A test-point GPIO** — if you have the watch opened / a dock, drive a pad and scope/LED it.

Mechanism: write the MSM8909 **TLMM** registers directly (GPIO config + output). Base and
pin number come from the `skipjack` DTS (doc 01). This is pure MMIO — works in SEC, before
clocks, before DRAM is fully mapped, as long as the TLMM is powered (it is, very early).

POST-code convention (`post_codes.h`): each milestone = a distinct blink count or Morse-ish
pattern. A hang freezes on the last pattern, telling you the furthest stage reached.

```
1 blip   = _ModuleEntryPoint reached
2 blips  = CPU/cache init done
3 blips  = memory map installed, DRAM OK
4 blips  = GIC + timer up
5 blips  = DXE dispatch started
long     = fatal fault handler (something threw)
```

## C. UART — most reliable once found

MSM8909 has BLSP UARTs. On a watch the lines are on **internal test points**, not a jack.
Finding them:

- Check the `skipjack` DTS `serial`/`uart` node for which BLSP UART is the debug console
  and its pin mux — that's the one AsteroidOS logs to.
- Probe test pads with a 1.8 V USB-UART adapter (watch GPIO is **1.8 V — do not use a
  3.3 V adapter directly**). TX/RX/GND; 115200 8N1 is the usual Qualcomm default.
- Once found, implement `SerialPortLib` against that BLSP base → every `DEBUG()` prints,
  and the UEFI console appears over serial.

Order of effort: try to locate UART early (it's the best channel), but don't *block* on
opening the watch — screen fills (A) + vibrator blink (B) get you moving immediately.

---

## Putting it together

Wire a `DEBUG_POST(code)` macro that does **all available channels at once**: set the
framebuffer color, blink the GPIO, and `DEBUG()` to UART. Then sprinkle it through SEC →
PEI → DXE. Whatever channel is alive on a given attempt, you get the stage number.
`post_codes.h` defines the codes; `fb_debug.c` / `gpio_blink.c` implement the sinks.
