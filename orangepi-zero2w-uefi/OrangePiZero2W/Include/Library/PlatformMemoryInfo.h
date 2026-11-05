/** @file
  Extra information about each entry of the platform virtual memory map,
  shared between PlatformLib and MemoryInitPeiLib.

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#ifndef PLATFORM_MEMORY_INFO_H_
#define PLATFORM_MEMORY_INFO_H_

typedef enum {
  PlatformMemDevice = 0,       // MMIO: mapped, not reported as memory
  PlatformMemSystem,           // Normal DRAM, free for UEFI and the OS
  PlatformMemReserved,         // DRAM the OS must never touch (TF-A)
  PlatformMemBootServices,     // DRAM used until ExitBootServices (the FD)
  PlatformMemTypeMax
} PLATFORM_MEMORY_TYPE;

typedef struct {
  PLATFORM_MEMORY_TYPE    Type;
  CONST CHAR8             *Name;
} PLATFORM_MEMORY_REGION_INFO;

/**
  Return the array describing each entry of the table returned by
  ArmPlatformGetVirtualMemoryMap(). Must be called after it.
**/
VOID
PlatformGetMemoryRegionInfo (
  OUT PLATFORM_MEMORY_REGION_INFO  **MemoryInfo
  );

#endif // PLATFORM_MEMORY_INFO_H_
