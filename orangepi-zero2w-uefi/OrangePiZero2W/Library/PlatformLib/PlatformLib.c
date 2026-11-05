/** @file
  ArmPlatformLib for the Orange Pi Zero 2W (Allwinner H618).

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#include <Library/ArmLib.h>
#include <Library/ArmPlatformLib.h>
#include <Pi/PiBootMode.h>
#include <Ppi/ArmMpCoreInfo.h>

//
// Four Cortex-A53 cores in a single cluster, MPIDR Aff0 = 0..3.
//
STATIC ARM_CORE_INFO  mCoreInfoTable[] = {
  { 0x0, },
  { 0x1, },
  { 0x2, },
  { 0x3, },
};

EFI_BOOT_MODE
ArmPlatformGetBootMode (
  VOID
  )
{
  return BOOT_WITH_FULL_CONFIGURATION;
}

/**
  Called by PeilessSec before memory is set up. U-Boot SPL has already
  initialised DRAM, clocks, pin muxing and UART0, and TF-A is running, so
  there is nothing to do here.
**/
RETURN_STATUS
ArmPlatformInitialize (
  IN  UINTN  MpId
  )
{
  return RETURN_SUCCESS;
}

STATIC
EFI_STATUS
GetMpCoreInfo (
  OUT UINTN          *CoreCount,
  OUT ARM_CORE_INFO  **ArmCoreTable
  )
{
  *CoreCount    = ARRAY_SIZE (mCoreInfoTable);
  *ArmCoreTable = mCoreInfoTable;
  return EFI_SUCCESS;
}

STATIC ARM_MP_CORE_INFO_PPI  mMpCoreInfoPpi = {
  GetMpCoreInfo
};

STATIC EFI_PEI_PPI_DESCRIPTOR  mPlatformPpiTable[] = {
  {
    EFI_PEI_PPI_DESCRIPTOR_PPI | EFI_PEI_PPI_DESCRIPTOR_TERMINATE_LIST,
    &gArmMpCoreInfoPpiGuid,
    &mMpCoreInfoPpi
  }
};

VOID
ArmPlatformGetPlatformPpiList (
  OUT UINTN                   *PpiListSize,
  OUT EFI_PEI_PPI_DESCRIPTOR  **PpiList
  )
{
  *PpiListSize = sizeof (mPlatformPpiTable);
  *PpiList     = mPlatformPpiTable;
}
