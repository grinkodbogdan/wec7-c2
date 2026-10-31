/** @file
  GPIO "blink" debug for TicWatch C2 (skipjack) — the LED substitute.

  The C2 has no notification LED, so we signal POST codes by toggling a GPIO the user can
  perceive: the vibration motor, the display backlight, or a test-point pad. Pure MMIO
  against the MSM8909 TLMM, so it works in SEC — before clocks, before DRAM is fully
  mapped — as long as the TLMM is powered (it is, very early).

  REFERENCE SCAFFOLDING. Replace TLMM base, pin number, and register offsets with values
  from docs/01-hardware-map.md (DTS 'tlmm' node + MSM8909 TLMM docs). Confirm which pin
  drives the vibrator/backlight before driving it.

  License intent: BSD-2-Clause-Patent. Linux DTS is a reference for addresses only.
**/

#include <Uefi.h>
#include <Library/IoLib.h>
#include <Library/TimerLib.h>   // MicroSecondDelay; before timer is up, use a busy loop
#include "post_codes.h"

//
// ---- Replace these from the hardware map (docs/01) ----
//
#define TLMM_BASE        0xDEAD0000u  // MSM8909 TLMM base (DTS 'tlmm' reg)
#define DEBUG_GPIO_NUM   0u           // pin for vibrator / backlight / test pad

//
// MSM8909 TLMM per-pin register layout (verify offsets against the SoC docs):
//   each GPIO has a 0x1000-byte block from TLMM_BASE.
//   +0x00 = CFG   (func sel, drive strength, pull, OE bit)
//   +0x04 = IN_OUT(bit1 = output value, bit0 = input value)
//
#define TLMM_PIN_STRIDE  0x1000u
#define GPIO_CFG(n)      (TLMM_BASE + (TLMM_PIN_STRIDE * (n)) + 0x00u)
#define GPIO_IN_OUT(n)   (TLMM_BASE + (TLMM_PIN_STRIDE * (n)) + 0x04u)

#define CFG_GPIO_MODE    (0u << 2)    // func sel 0 = plain GPIO
#define CFG_OE           (1u << 9)    // output enable
#define OUT_VAL_BIT      (1u << 1)

STATIC VOID
GpioConfigOutput (
  IN UINTN Pin
  )
{
  // Plain GPIO, output enabled, 2mA, no pull. Adjust drive strength bits per docs.
  MmioWrite32 (GPIO_CFG (Pin), CFG_GPIO_MODE | CFG_OE);
}

STATIC VOID
GpioSet (
  IN UINTN   Pin,
  IN BOOLEAN High
  )
{
  UINT32 v = MmioRead32 (GPIO_IN_OUT (Pin));
  if (High) {
    v |= OUT_VAL_BIT;
  } else {
    v &= ~OUT_VAL_BIT;
  }
  MmioWrite32 (GPIO_IN_OUT (Pin), v);
}

//
// Crude delay that does not depend on the architected timer being up yet. Tune the loop
// count on hardware. Once PostGicTimer is reached you can switch to MicroSecondDelay().
//
STATIC VOID
CrudeDelay (
  IN UINTN Loops
  )
{
  volatile UINTN i;
  for (i = 0; i < Loops; i++) {
    __asm__ __volatile__("nop");
  }
}

/**
  Blip the debug GPIO Count times: on-200ms, off-200ms, with a longer gap after the
  group so successive codes are distinguishable to a human watching/feeling the watch.
**/
VOID
GpioBlink (
  IN UINTN Count
  )
{
  UINTN i;

  GpioConfigOutput (DEBUG_GPIO_NUM);
  for (i = 0; i < Count; i++) {
    GpioSet (DEBUG_GPIO_NUM, TRUE);
    CrudeDelay (2000000);           // ~on   (tune)
    GpioSet (DEBUG_GPIO_NUM, FALSE);
    CrudeDelay (2000000);           // ~off  (tune)
  }
  CrudeDelay (8000000);             // inter-code gap
}

/**
  Fatal: hold the GPIO on in a long continuous pattern so a hang in the fault handler is
  unmistakable (steady vibration / backlight on).
**/
VOID
GpioBlinkFatal (
  VOID
  )
{
  GpioConfigOutput (DEBUG_GPIO_NUM);
  for (;;) {
    GpioSet (DEBUG_GPIO_NUM, TRUE);
    CrudeDelay (12000000);
    GpioSet (DEBUG_GPIO_NUM, FALSE);
    CrudeDelay (2000000);
  }
}
