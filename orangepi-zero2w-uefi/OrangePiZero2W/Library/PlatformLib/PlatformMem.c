/** @file
  Memory map for the Orange Pi Zero 2W (Allwinner H618).

  Physical layout seen by UEFI:

    0x0000_0000 - 0x3FFF_FFFF   SRAM + MMIO (device memory)
    0x4000_0000 - 0x401F_FFFF   TF-A BL31 (reserved, never given to the OS)
    0x4020_0000 - FD base       DRAM
    0x4A00_0000 - FD end        This firmware image (freed at ExitBootServices)
    FD end      - 0x7FFF_FFFF   DRAM (the PeilessSec UEFI region is at the top)
    0x8000_0000 - DRAM end      DRAM above 1 GiB, size detected at boot

  The board ships with 1, 1.5, 2 or 4 GiB. PcdSystemMemorySize is the 1 GiB
  every variant has, and everything above it is detected here by probing.

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#include <Library/ArmPlatformLib.h>
#include <Library/BaseLib.h>
#include <Library/DebugLib.h>
#include <Library/PcdLib.h>
#include <Library/PlatformMemoryInfo.h>
#include <H618.h>

#define MAX_REGIONS  7

//
// DRAM is probed in 256 MiB slots. Each slot is tested at an offset of
// 128 MiB, which keeps the probe away from TF-A (bottom of DRAM), the
// firmware image (0x4A00_0000) and the UEFI stack/HOB region at the top of
// the first GiB (0x7C00_0000 - 0x7FFF_FFFF).
//
#define PROBE_SLOT_SIZE    SIZE_256MB
#define PROBE_SLOT_OFFSET  SIZE_128MB
#define PROBE_MAX_SLOTS    16                 // 16 x 256 MiB = 4 GiB
#define PROBE_MAGIC        0x5A0E2000U

STATIC ARM_MEMORY_REGION_DESCRIPTOR  mVirtualMemoryTable[MAX_REGIONS];
STATIC PLATFORM_MEMORY_REGION_INFO   mVirtualMemoryInfo[MAX_REGIONS];

UINT64  mDramSize;

/**
  Find how much DRAM is fitted.

  The H616/H618 DRAM controller ignores address bits above the fitted size,
  so on a smaller board the upper slots alias the lower ones. Every slot gets
  a different value, written from the top down: an aliased slot is then
  overwritten by the lower slot it maps onto and reads back the wrong value.

  This runs from MemoryPeim() with the MMU (and therefore the data cache) off,
  so every access goes straight to DRAM. Original contents are restored.
**/
STATIC
UINT64
ProbeDramSize (
  VOID
  )
{
  UINT32            Saved[PROBE_MAX_SLOTS];
  volatile UINT32   *Ptr;
  UINT64            MaxSlots;
  UINTN             Index;
  UINTN             Found;

  MaxSlots = FixedPcdGet64 (PcdDramMaxSize) / PROBE_SLOT_SIZE;
  if (MaxSlots > PROBE_MAX_SLOTS) {
    MaxSlots = PROBE_MAX_SLOTS;
  }

  for (Index = 0; Index < MaxSlots; Index++) {
    Ptr          = (volatile UINT32 *)(UINTN)(H618_DRAM_BASE + Index * PROBE_SLOT_SIZE + PROBE_SLOT_OFFSET);
    Saved[Index] = *Ptr;
  }

  for (Index = MaxSlots; Index > 0; Index--) {
    Ptr  = (volatile UINT32 *)(UINTN)(H618_DRAM_BASE + (Index - 1) * PROBE_SLOT_SIZE + PROBE_SLOT_OFFSET);
    *Ptr = PROBE_MAGIC + (UINT32)(Index - 1);
    MemoryFence ();
  }

  Found = 0;
  for (Index = 0; Index < MaxSlots; Index++) {
    Ptr = (volatile UINT32 *)(UINTN)(H618_DRAM_BASE + Index * PROBE_SLOT_SIZE + PROBE_SLOT_OFFSET);
    if (*Ptr != PROBE_MAGIC + (UINT32)Index) {
      break;
    }

    Found++;
  }

  //
  // Restore in the reverse order of the writes. Aliased slots share a
  // location and were saved before any write, so all saved values agree.
  //
  for (Index = 0; Index < MaxSlots; Index++) {
    Ptr  = (volatile UINT32 *)(UINTN)(H618_DRAM_BASE + Index * PROBE_SLOT_SIZE + PROBE_SLOT_OFFSET);
    *Ptr = Saved[Index];
  }

  MemoryFence ();

  return (UINT64)Found * PROBE_SLOT_SIZE;
}

STATIC
VOID
AddRegion (
  IN OUT UINTN                 *Index,
  IN     UINT64                Base,
  IN     UINT64                Length,
  IN     ARM_MEMORY_REGION_ATTRIBUTES  Attributes,
  IN     PLATFORM_MEMORY_TYPE  Type,
  IN     CONST CHAR8           *Name
  )
{
  if (Length == 0) {
    return;
  }

  ASSERT (*Index < MAX_REGIONS - 1);

  mVirtualMemoryTable[*Index].PhysicalBase = Base;
  mVirtualMemoryTable[*Index].VirtualBase  = Base;
  mVirtualMemoryTable[*Index].Length       = Length;
  mVirtualMemoryTable[*Index].Attributes   = Attributes;
  mVirtualMemoryInfo[*Index].Type          = Type;
  mVirtualMemoryInfo[*Index].Name          = Name;
  (*Index)++;
}

/**
  Return the Virtual Memory Map of the platform, used by MemoryInitPeiLib to
  build the memory HOBs and configure the MMU.
**/
VOID
ArmPlatformGetVirtualMemoryMap (
  OUT ARM_MEMORY_REGION_DESCRIPTOR  **VirtualMemoryMap
  )
{
  UINTN   Index;
  UINT64  DramBase;
  UINT64  FirstGbEnd;
  UINT64  TfaEnd;
  UINT64  FdBase;
  UINT64  FdEnd;

  ASSERT (VirtualMemoryMap != NULL);

  DramBase   = FixedPcdGet64 (PcdSystemMemoryBase);
  FirstGbEnd = DramBase + FixedPcdGet64 (PcdSystemMemorySize);
  TfaEnd     = DramBase + FixedPcdGet64 (PcdTfaReservedSize);
  FdBase     = FixedPcdGet64 (PcdFdBaseAddress);
  FdEnd      = FdBase + FixedPcdGet32 (PcdFdSize);

  ASSERT (DramBase == H618_DRAM_BASE);
  ASSERT (FdBase >= TfaEnd && FdEnd <= FirstGbEnd);

  if (mDramSize == 0) {
    mDramSize = ProbeDramSize ();
    if (mDramSize < FixedPcdGet64 (PcdSystemMemorySize)) {
      //
      // Should not happen on a real board: the 1 GiB below is already in use
      // by PeilessSec. Do not shrink it, just report the oddity.
      //
      DEBUG ((DEBUG_ERROR, "DRAM probe found only 0x%lx bytes\n", mDramSize));
      mDramSize = FixedPcdGet64 (PcdSystemMemorySize);
    }
  }

  DEBUG ((DEBUG_INFO, "DRAM: %lu MiB\n", mDramSize / SIZE_1MB));

  Index = 0;
  AddRegion (&Index, H618_MMIO_BASE, H618_MMIO_SIZE, ARM_MEMORY_REGION_ATTRIBUTE_DEVICE, PlatformMemDevice, "SoC MMIO");
  AddRegion (&Index, DramBase, TfaEnd - DramBase, ARM_MEMORY_REGION_ATTRIBUTE_WRITE_BACK, PlatformMemReserved, "TF-A BL31");
  AddRegion (&Index, TfaEnd, FdBase - TfaEnd, ARM_MEMORY_REGION_ATTRIBUTE_WRITE_BACK, PlatformMemSystem, "DRAM below FD");
  AddRegion (&Index, FdBase, FdEnd - FdBase, ARM_MEMORY_REGION_ATTRIBUTE_WRITE_BACK, PlatformMemBootServices, "Firmware (FD)");
  AddRegion (&Index, FdEnd, FirstGbEnd - FdEnd, ARM_MEMORY_REGION_ATTRIBUTE_WRITE_BACK, PlatformMemSystem, "DRAM < 1 GiB");
  AddRegion (
    &Index,
    FirstGbEnd,
    DramBase + mDramSize - FirstGbEnd,
    ARM_MEMORY_REGION_ATTRIBUTE_WRITE_BACK,
    PlatformMemSystem,
    "DRAM > 1 GiB"
    );

  // Zero-filled terminator (the arrays are static, so already zero).
  ASSERT (Index < MAX_REGIONS);

  *VirtualMemoryMap = mVirtualMemoryTable;
}

VOID
PlatformGetMemoryRegionInfo (
  OUT PLATFORM_MEMORY_REGION_INFO  **MemoryInfo
  )
{
  ASSERT (MemoryInfo != NULL);
  *MemoryInfo = mVirtualMemoryInfo;
}
