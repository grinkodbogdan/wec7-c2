/** @file
  Common definitions for the Orange Pi Zero 2W ACPI tables.

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#ifndef ACPI_TABLES_H_
#define ACPI_TABLES_H_

#include <IndustryStandard/Acpi.h>
#include <H618.h>

#define EFI_ACPI_OEM_ID            {'O','P','I','Z','2','W'}
#define EFI_ACPI_OEM_TABLE_ID      SIGNATURE_64 ('H','6','1','8',' ',' ',' ',' ')
#define EFI_ACPI_OEM_REVISION      0x00000001
#define EFI_ACPI_CREATOR_ID        SIGNATURE_32 ('E','D','K','2')
#define EFI_ACPI_CREATOR_REVISION  0x00000001

#define ACPI_HEADER(Signature, Type, Revision) {                  \
    Signature,                      /* UINT32  Signature */       \
    sizeof (Type),                  /* UINT32  Length */          \
    Revision,                       /* UINT8   Revision */        \
    0,                              /* UINT8   Checksum */        \
    EFI_ACPI_OEM_ID,                /* UINT8   OemId[6] */        \
    EFI_ACPI_OEM_TABLE_ID,          /* UINT64  OemTableId */      \
    EFI_ACPI_OEM_REVISION,          /* UINT32  OemRevision */     \
    EFI_ACPI_CREATOR_ID,            /* UINT32  CreatorId */       \
    EFI_ACPI_CREATOR_REVISION       /* UINT32  CreatorRevision */ \
  }

#endif // ACPI_TABLES_H_
