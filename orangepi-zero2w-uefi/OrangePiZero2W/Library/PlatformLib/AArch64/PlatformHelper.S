/** @file
  Early (pre-stack) platform hook for the Orange Pi Zero 2W.

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#include <AsmMacroLib.h>

//
// Called by PeilessSec's entry point before a stack exists. U-Boot SPL and
// TF-A have already done all early board setup, so nothing is needed.
// Must not touch the stack or x0-x3.
//
ASM_FUNC(ArmPlatformPeiBootAction)
  ret
