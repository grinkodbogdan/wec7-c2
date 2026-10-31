/** @file
  Framebuffer ("screen") debug for TicWatch C2 (skipjack) early UEFI bring-up.

  Paints the linear framebuffer the bootloader (ABL continuous-splash) already set up,
  so you get visual signal before any display/GOP driver exists.

  REFERENCE SCAFFOLDING — not a drop-in. Replace every placeholder with real values
  from docs/01-hardware-map.md (DTS mdss/cont_splash + /sys/class/graphics/fb0/*).

  License intent: BSD-2-Clause-Patent.
**/

#include <Uefi.h>
#include <Library/BaseLib.h>
#include <Library/IoLib.h>          // MmioRead/Write if you need to re-poke backlight
#include "post_codes.h"

//
// ---- Replace these from the hardware map (docs/01) ----
//
#define FB_BASE     0xDEAD0000u     // physical framebuffer base from cont_splash / DTS
#define FB_WIDTH    360u            // /sys/class/graphics/fb0/virtual_size (w)
#define FB_HEIGHT   360u            // /sys/class/graphics/fb0/virtual_size (h)
#define FB_STRIDE   (FB_WIDTH * 4u) // bytes per row; confirm /sys/.../stride
#define FB_BPP      32u             // bits per pixel; confirm /sys/.../bits_per_pixel
//
// Panel byte order. MSM8909 splash is commonly BGRA/xRGB little-endian. If colors look
// swapped on hardware, flip this to 1 and rebuild — a swapped color is still useful signal.
//
#define FB_SWAP_RB  0

STATIC volatile UINT32 *mFb = (volatile UINT32 *)(UINTN)FB_BASE;

STATIC UINT32
PackPixel (
  IN UINT32 Rgb   // 0x00RRGGBB
  )
{
  UINT32 R = (Rgb >> 16) & 0xFF;
  UINT32 G = (Rgb >>  8) & 0xFF;
  UINT32 B = (Rgb >>  0) & 0xFF;
#if FB_SWAP_RB
  return (0xFFu << 24) | (B << 16) | (G << 8) | R;  // ARGB with R/B swapped
#else
  return (0xFFu << 24) | (R << 16) | (G << 8) | B;  // ARGB
#endif
}

/**
  Fill the whole framebuffer with one color. The simplest possible POST indicator:
  the last color painted before a hang tells you the furthest stage reached.
**/
VOID
FbDebugFill (
  IN UINT32 Rgb
  )
{
  UINT32 px = PackPixel (Rgb);
  UINTN  i;
  UINTN  count = (FB_STRIDE / 4u) * FB_HEIGHT;

  if (mFb == NULL) {
    return;
  }
  for (i = 0; i < count; i++) {
    mFb[i] = px;
  }
}

//
// Minimal 8x8 bitmap font — only the glyphs needed to print hex ("0123456789ABCDEF"),
// plus space and 'x'. Fill in the bitmaps from any public-domain 8x8 font. Each glyph is
// 8 bytes, one byte per row, MSB = leftmost pixel.
//
STATIC CONST UINT8 mFont8x8['F' + 1][8] = {
  // ... populate ' ', '0'..'9', 'A'..'F', 'x', 'X' from a PD 8x8 font ...
  [' '] = {0,0,0,0,0,0,0,0},
  ['0'] = {0x3C,0x66,0x6E,0x76,0x66,0x66,0x3C,0x00},
  // (remaining glyphs omitted in this scaffold)
};

STATIC VOID
DrawGlyph (
  IN UINTN X,
  IN UINTN Y,
  IN CHAR8 C,
  IN UINT32 Rgb
  )
{
  UINT32 px = PackPixel (Rgb);
  UINTN  row, col;

  if ((UINT8)C > 'F') {
    C = ' ';
  }
  for (row = 0; row < 8; row++) {
    UINT8 bits = mFont8x8[(UINT8)C][row];
    for (col = 0; col < 8; col++) {
      if (bits & (0x80u >> col)) {
        UINTN px_x = X + col;
        UINTN px_y = Y + row;
        if (px_x < FB_WIDTH && px_y < FB_HEIGHT) {
          mFb[px_y * (FB_STRIDE / 4u) + px_x] = px;
        }
      }
    }
  }
}

/**
  Print an ASCII string at (X,Y) in white. Call only after FbDebugFill proves the
  framebuffer pointer/geometry are right.
**/
VOID
FbDebugText (
  IN UINTN        X,
  IN UINTN        Y,
  IN CONST CHAR8 *Str
  )
{
  if (mFb == NULL || Str == NULL) {
    return;
  }
  while (*Str != '\0') {
    DrawGlyph (X, Y, *Str, 0x00FFFFFF);
    X += 8;
    Str++;
  }
}
