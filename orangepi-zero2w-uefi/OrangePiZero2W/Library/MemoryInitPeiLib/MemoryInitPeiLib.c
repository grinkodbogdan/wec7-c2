/** @file
  Build the memory HOBs from the platform memory map and turn on the MMU.

  Based on the Raspberry Pi MemoryInitPeiLib from edk2-platforms.

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#include <PiPei.h>

#include <Library/ArmMmuLib.h>
#include <Library/ArmPlatformLib.h>
#include <Library/DebugLib.h>
#include <Library/HobLib.h>
#include <Library/PcdLib.h>
#include <Library/PlatformMemoryInfo.h>

VOID
BuildMemoryTypeInformationHob (
  VOID
  );

STATIC
VOID
AddSystemMemory (
  IN ARM_MEMORY_REGION_DESCRIPTOR  *Desc
  )
{
  BuildResourceDescriptorHob (
    EFI_RESOURCE_SYSTEM_MEMORY,
    EFI_RESOURCE_ATTRIBUTE_PRESENT |
    EFI_RESOURCE_ATTRIBUTE_INITIALIZED |
    EFI_RESOURCE_ATTRIBUTE_WRITE_COMBINEABLE |
    EFI_RESOURCE_ATTRIBUTE_WRITE_THROUGH_CACHEABLE |
    EFI_RESOURCE_ATTRIBUTE_WRITE_BACK_CACHEABLE |
    EFI_RESOURCE_ATTRIBUTE_WRITE_PROTECTABLE |
    EFI_RESOURCE_ATTRIBUTE_EXECUTION_PROTECTABLE |
    EFI_RESOURCE_ATTRIBUTE_TESTED,
    Desc->PhysicalBase,
    Desc->Length
    );
}

EFI_STATUS
EFIAPI
MemoryPeim (
  IN EFI_PHYSICAL_ADDRESS  UefiMemoryBase,
  IN UINT64                UefiMemorySize
  )
{
  ARM_MEMORY_REGION_DESCRIPTOR  *MemoryTable;
  PLATFORM_MEMORY_REGION_INFO   *MemoryInfo;
  UINTN                         Index;
  RETURN_STATUS                 Status;

  ArmPlatformGetVirtualMemoryMap (&MemoryTable);
  PlatformGetMemoryRegionInfo (&MemoryInfo);

  for (Index = 0; MemoryTable[Index].Length != 0; Index++) {
    DEBUG ((
      DEBUG_INFO,
      "%a: 0x%011lx - 0x%011lx\n",
      MemoryInfo[Index].Name,
      MemoryTable[Index].PhysicalBase,
      MemoryTable[Index].PhysicalBase + MemoryTable[Index].Length - 1
      ));

    switch (MemoryInfo[Index].Type) {
      case PlatformMemSystem:
        AddSystemMemory (&MemoryTable[Index]);
        break;

      case PlatformMemReserved:
        AddSystemMemory (&MemoryTable[Index]);
        BuildMemoryAllocationHob (
          MemoryTable[Index].PhysicalBase,
          MemoryTable[Index].Length,
          EfiReservedMemoryType
          );
        break;

      case PlatformMemBootServices:
        AddSystemMemory (&MemoryTable[Index]);
        BuildMemoryAllocationHob (
          MemoryTable[Index].PhysicalBase,
          MemoryTable[Index].Length,
          EfiBootServicesData
          );
        break;

      case PlatformMemDevice:
      default:
        break;
    }
  }

  Status = ArmConfigureMmu (MemoryTable, NULL, NULL);
  if (RETURN_ERROR (Status)) {
    DEBUG ((DEBUG_ERROR, "Failed to enable the MMU: %r\n", Status));
    ASSERT_RETURN_ERROR (Status);
  }

  if (FeaturePcdGet (PcdPrePiProduceMemoryTypeInformationHob)) {
    BuildMemoryTypeInformationHob ();
  }

  return EFI_SUCCESS;
}
