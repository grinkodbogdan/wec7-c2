/** @file
  Allwinner H616/H618 SoC definitions used by the Orange Pi Zero 2W port.

  Values come from the mainline Linux/U-Boot device tree
  (arch/arm64/boot/dts/allwinner/sun50i-h616.dtsi).

  SPDX-License-Identifier: BSD-2-Clause-Patent
**/

#ifndef H618_H_
#define H618_H_

//
// Address map
//
#define H618_MMIO_BASE           0x00000000ULL  // SRAM + all peripherals
#define H618_MMIO_SIZE           0x40000000ULL
#define H618_DRAM_BASE           0x40000000ULL

//
// UART0 (DesignWare APB UART, 16550 compatible, 32-bit register stride)
//
#define H618_UART0_BASE          0x05000000
#define H618_UART0_SIZE          0x400
#define H618_UART0_GSIV          (32 + 0)       // SPI 0
#define H618_UART_CLOCK_HZ       24000000       // APB2 = OSC24M

//
// GIC-400 (GICv2)
//
#define H618_GICD_BASE           0x03021000
#define H618_GICC_BASE           0x03022000
#define H618_GICH_BASE           0x03024000
#define H618_GICV_BASE           0x03026000

//
// SD card controller (not driven by this firmware yet, described for later)
//
#define H618_MMC0_BASE           0x04020000
#define H618_MMC0_GSIV           (32 + 35)      // SPI 35

#define H618_CPU_COUNT           4

#endif // H618_H_
