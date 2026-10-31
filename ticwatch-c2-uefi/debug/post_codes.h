/** @file
  POST codes and a multi-channel DEBUG_POST() macro for the TicWatch C2 (skipjack)
  EDK2/UEFI bring-up.

  Reference scaffolding, not a drop-in. All addresses are placeholders (0xDEADxxxx):
  replace them with real values extracted per docs/01-hardware-map.md.

  License intent: BSD-2-Clause-Patent (EDK2-compatible). Do NOT paste GPL kernel code
  here; use the Linux DTS/sources only to look up addresses (project rule 4).
**/

#ifndef SKIPJACK_POST_CODES_H_
#define SKIPJACK_POST_CODES_H_

//
// Milestone POST codes. Keep them small and monotonic so a GPIO blink count and a
// framebuffer color both read as "how far did we get before the hang".
//
typedef enum {
  PostEntry          = 1,  // _ModuleEntryPoint reached
  PostCpuInit        = 2,  // CPU / cache / MMU-off state sane
  PostMemoryMap      = 3,  // ArmPlatformGetVirtualMemoryMap installed, DRAM usable
  PostGicTimer       = 4,  // GIC + architected timer up
  PostDxeDispatch    = 5,  // DXE core dispatching
  PostSerialUp       = 6,  // SerialPortLib live (UART prints from here on)
  PostGopUp          = 7,  // Graphics Output Protocol over framebuffer
  PostStorageUp      = 8,  // eMMC visible
  PostFatal          = 0xF // fatal fault handler (long blink / red screen)
} SKIPJACK_POST_CODE;

//
// Per-code framebuffer color (0x00RRGGBB; byte order fixed up in fb_debug.c to match
// the panel's actual pixel format from /sys/class/graphics/fb0/*).
//
#define POST_COLOR_ENTRY      0x000000FF  // blue
#define POST_COLOR_CPU        0x0000FFFF  // cyan
#define POST_COLOR_MEMMAP     0x0000FF00  // green
#define POST_COLOR_GICTIMER   0x00FFFF00  // yellow
#define POST_COLOR_DXE        0x00FF8000  // orange
#define POST_COLOR_FATAL      0x00FF0000  // red

//
// Sinks implemented in fb_debug.c and gpio_blink.c. Each is a no-op if that channel
// isn't available yet, so DEBUG_POST() is always safe to call.
//
VOID FbDebugFill   (UINT32 Rgb);                 // full-screen color fill
VOID FbDebugText   (UINTN X, UINTN Y, CONST CHAR8 *Str); // 8x8 font (once fills work)
VOID GpioBlink     (UINTN Count);                // blip vibrator/GPIO Count times
VOID GpioBlinkFatal (VOID);                      // long continuous pattern

//
// Fire every channel at once. Whatever is alive on this boot attempt, you get signal.
// Pair the numeric code (GPIO blink) with a color (screen) and a UART string.
//
#define DEBUG_POST(Code, Color, Str)        \
  do {                                      \
    FbDebugFill (Color);                    \
    GpioBlink   ((UINTN)(Code));            \
    DEBUG ((DEBUG_INFO, "[POST %u] %a\n", (UINT32)(Code), (Str))); \
  } while (FALSE)

#endif // SKIPJACK_POST_CODES_H_
