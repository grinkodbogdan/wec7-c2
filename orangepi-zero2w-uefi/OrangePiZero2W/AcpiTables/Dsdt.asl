/** @file
  Differentiated System Description Table (DSDT) for the Orange Pi Zero 2W.

  Only what is needed for an OS to boot to a serial console: the four CPUs
  and UART0. Storage, USB, display and Wi-Fi need drivers that do not exist
  yet, so they are not described here.

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

DefinitionBlock ("DsdtTable.aml", "DSDT", 2, "OPIZ2W", "H618    ", 1)
{
  Scope (\_SB)
  {
    //
    // Cortex-A53 cores. _UID must match AcpiCpuUid in the MADT.
    //
    Device (CPU0) { Name (_HID, "ACPI0007") Name (_UID, 0) }
    Device (CPU1) { Name (_HID, "ACPI0007") Name (_UID, 1) }
    Device (CPU2) { Name (_HID, "ACPI0007") Name (_UID, 2) }
    Device (CPU3) { Name (_HID, "ACPI0007") Name (_UID, 3) }

    //
    // UART0: Synopsys DesignWare APB UART, 24 MHz clock, 32-bit registers
    // 4 bytes apart. Linux matches it through PRP0001 + the compatible
    // string; Windows uses SPCR/DBG2 for the console and debugger instead.
    //
    Device (URT0)
    {
      Name (_HID, "PRP0001")
      Name (_UID, 0)
      Name (_CRS, ResourceTemplate () {
        Memory32Fixed (ReadWrite, 0x05000000, 0x00000400)
        Interrupt (ResourceConsumer, Level, ActiveHigh, Exclusive) { 32 }
      })
      Name (_DSD, Package () {
        ToUUID ("daffd814-6eba-4d8c-8a91-bc9bbf4aa301"),
        Package () {
          Package () { "compatible", "snps,dw-apb-uart" },
          Package () { "clock-frequency", 24000000 },
          Package () { "reg-shift", 2 },
          Package () { "reg-io-width", 4 },
        }
      })
      Method (_STA) { Return (0xF) }
    }
  }
}
