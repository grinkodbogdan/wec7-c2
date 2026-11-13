/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * PCIe root complex 0 of the MSM8996, which carries the Mi 5's Wi-Fi chip
 * (QCA6174, "Rome"): power, clocks, the 14 nm QMP PHY, link training, and
 * the endpoint's configuration space and first BAR.
 *
 * The sequence, register offsets and PHY values follow the phone's own
 * kernel, drivers/pci/host/pci-msm.c (LineageOS android_kernel_xiaomi_
 * msm8996), Copyright (c) 2014-2017 The Linux Foundation, GPL-2.0, as
 * msm_pcie_enable() runs it for RC0 with the device tree's settings
 * (qcom,pcie-phy-ver = <3>, qcom,common-phy, perst-gpio = <&tlmm 35 0>).
 * Hence this file's licence: it lives apart from the BSD-licensed tree,
 * as sys/dev/drm/msm does.
 *
 *   sysctl hw.mi5.pcie_up=1	bring the link up (once)
 *   sysctl hw.mi5.pcie		what happened, step by step
 *   sysctl hw.mi5.wlan_id	the endpoint's vendor:device, 0 if none
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/sysctl.h>
#include <sys/lock.h>
#include <sys/kthread.h>

#include <vm/vm.h>
#include <vm/pmap.h>

#include <machine/md_var.h>
#include <machine/vmparam.h>

#include "pcie_msm8996.h"

void	athp_dfly_crumb(const char *);

/* GCC */
#define	GCC			0x00300000UL
#define	GCC_PCIE_0_SLV_AXI_CBCR	(GCC + 0x6b008)
#define	GCC_PCIE_0_MSTR_AXI_CBCR (GCC + 0x6b00c)
#define	GCC_PCIE_0_CFG_AHB_CBCR	(GCC + 0x6b010)
#define	GCC_PCIE_0_AUX_CBCR	(GCC + 0x6b014)
#define	GCC_PCIE_0_PIPE_CBCR	(GCC + 0x6b018)
#define	GCC_PCIE_0_GDSCR	(GCC + 0x6b004)
#define	GCC_PCIE_AUX_CMD_RCGR	(GCC + 0x6c000)
#define	GCC_PCIE_0_PHY_BCR	(GCC + 0x6c01c)
#define	GCC_PCIE_PHY_BCR	(GCC + 0x6f000)
#define	GCC_PCIE_PHY_CFG_AHB_CBCR (GCC + 0x6f004)
#define	GCC_PCIE_PHY_AUX_CBCR	(GCC + 0x6f008)
#define	GCC_PCIE_PHY_NOCSR_COM_PHY_BCR (GCC + 0x6f00c)
#define	GCC_PCIE_PHY_COM_BCR	(GCC + 0x6f014)
#define	GCC_SMMU_AGGRE0_AXI_CBCR (GCC + 0x81014)
#define	GCC_AGGRE0_NOC_GDSCR	(GCC + 0x81004)
#define	GCC_SMMU_AGGRE0_AHB_CBCR (GCC + 0x81018)
/* the aggre0 NoC's own clocks (Linux's bus driver turns them on for a vote) */
#define	GCC_AGGRE0_SNOC_AXI_CBCR (GCC + 0x81008)
#define	GCC_AGGRE0_CNOC_AHB_CBCR (GCC + 0x8100c)
/* anoc0's SMMU (ARM SMMU v2), in front of the PCIe masters */
#define	ANOC0_SMMU		0x01640000UL
#define	SMMU_SCR0		0x000
#define	SMMU_SIDR0		0x020
#define	SMMU_SGFSR		0x048
#define	SMMU_SGFSYNR1		0x054

static void	smmu_bypass(void);
#define	GCC_PCIE_CLKREF_EN	(GCC + 0x88010)

/* TLMM */
#define	TLMM_CFG(n)		(0x01010000UL + 0x1000 * (n))
#define	TLMM_IO(n)		(0x01010000UL + 0x1000 * (n) + 4)
#define	PERST_GPIO		35
#define	BOOTSTRAP_GPIO		46

/* RC0 */
#define	PARF			0x00600000UL
#define	PHY			0x00034000UL
#define	DBI			0x0c000000UL
#define	ELBI			0x0c000f20UL
#define	CONF			0x0c100000UL
#define	BARS			0x0c300000UL
#define	BARS_SIZE		0x00d00000UL

#define	PARF_SYS_CTRL		0x000
#define	PARF_PHY_CTRL		0x040
#define	PARF_DBI_BASE_ADDR	0x168
#define	PARF_SLV_ADDR_SPACE_SIZE 0x16c
#define	PARF_MHI_CLOCK_RESET_CTRL 0x174
#define	PARF_LTSSM		0x1b0
#define	PARF_SID_OFFSET		0x234
#define	PARF_BDF_TRANSLATE_CFG	0x24c

#define	ELBI_SYS_STTS		0x08
#define	XMLH_LINK_UP		0x400

#define	DBI_COMMAND_STATUS	0x04
#define	DBI_BUSNUMBERS		0x18
#define	DBI_MEMORY_BASE_LIMIT	0x20
#define	DBI_LINK_STATUS		0x80
#define	DBI_DEVICE_CONTROL_STATUS 0x78
#define	DBI_ACK_F_ASPM_CTRL	0x70c
#define	DBI_AUX_CLK_FREQ	0xb40
#define	IATU_VIEWPORT		0x900
#define	IATU_CTRL1		0x904
#define	IATU_CTRL2		0x908
#define	IATU_LBAR		0x90c
#define	IATU_UBAR		0x910
#define	IATU_LAR		0x914
#define	IATU_LTAR		0x918
#define	IATU_UTAR		0x91c
#define	IATU_TYPE_MEM		0x00
#define	IATU_TYPE_CFG0		0x04

/* the 14 nm QMP PHY: common block, then port 0's TX, RX and PCS */
#define	COM_BG_TIMER		0x00c
#define	COM_SSC_EN_CENTER	0x010
#define	COM_SSC_ADJ_PER1	0x014
#define	COM_SSC_ADJ_PER2	0x018
#define	COM_SSC_PER1		0x01c
#define	COM_SSC_PER2		0x020
#define	COM_SSC_STEP_SIZE1	0x024
#define	COM_SSC_STEP_SIZE2	0x028
#define	COM_BIAS_EN_CLKBUFLR_EN	0x034
#define	COM_CLK_ENABLE1		0x038
#define	COM_SYS_CLK_CTRL	0x03c
#define	COM_SYSCLK_BUF_ENABLE	0x040
#define	COM_PLL_IVCO		0x048
#define	COM_LOCK_CMP1_MODE0	0x04c
#define	COM_LOCK_CMP2_MODE0	0x050
#define	COM_LOCK_CMP3_MODE0	0x054
#define	COM_BG_TRIM		0x070
#define	COM_CLK_EP_DIV		0x074
#define	COM_CP_CTRL_MODE0	0x078
#define	COM_PLL_RCTRL_MODE0	0x084
#define	COM_PLL_CCTRL_MODE0	0x090
#define	COM_SYSCLK_EN_SEL	0x0ac
#define	COM_RESCODE_DIV_NUM	0x0c4
#define	COM_LOCK_CMP_EN		0x0c8
#define	COM_DEC_START_MODE0	0x0d0
#define	COM_DIV_FRAC_START1_MODE0 0x0dc
#define	COM_DIV_FRAC_START2_MODE0 0x0e0
#define	COM_DIV_FRAC_START3_MODE0 0x0e4
#define	COM_INTEGLOOP_GAIN0_MODE0 0x108
#define	COM_INTEGLOOP_GAIN1_MODE0 0x10c
#define	COM_VCO_TUNE_MAP	0x128
#define	COM_VCO_TUNE_TIMER1	0x144
#define	COM_VCO_TUNE_TIMER2	0x148
#define	COM_CLK_SELECT		0x174
#define	COM_HSCLK_SEL		0x178
#define	COM_CORECLK_DIV		0x184
#define	COM_CORE_CLK_EN		0x18c
#define	COM_CMN_CONFIG		0x194
#define	COM_SVS_MODE_CLK_SEL	0x19c
#define	PCIE_COM_SW_RESET	0x400
#define	PCIE_COM_POWER_DOWN_CONTROL 0x404
#define	PCIE_COM_START_CONTROL	0x408
#define	PCIE_COM_PCS_READY_STATUS 0x448

/* port n = 0 with a common PHY: TX at 0x1000, RX 0x1200, PCS 0x1400 */
#define	TX0			0x1000
#define	RX0			0x1200
#define	PCS0			0x1400

#define	PM8994_SID		0
#define	PM8994_GPIO(n)		(0xc000 + 0x100 * ((n) - 1))

/* RPM resources (type, then keys), as little-endian ASCII */
#define	RPM_LDOA		0x616f646c	/* "ldoa" */
#define	RPM_MISC_CLK		0x306b6c63	/* "clk0" */
#define	RPM_KEY_SWEN		0x6e657773	/* "swen" */
#define	RPM_KEY_UV		0x00007675	/* "uv" */
#define	RPM_KEY_ENAB		0x62616e45	/* "Enab" */
#define	RPM_LN_BB_CLK		8

static char pcie_status[1024];
static int wlan_id;
static int pcie_done;
static struct lock pcie_lock = LOCK_INITIALIZER("pcie0", 0, 0);

SYSCTL_DECL(_hw_mi5);
SYSCTL_STRING(_hw_mi5, OID_AUTO, pcie, CTLFLAG_RD, pcie_status, 0,
    "PCIe RC0 bring-up, step by step");
SYSCTL_INT(_hw_mi5, OID_AUTO, wlan_id, CTLFLAG_RD, &wlan_id, 0,
    "PCIe RC0 endpoint vendor:device (0x168c003e: QCA6174)");

static void
st(const char *fmt, ...)
{
	char line[160];
	__va_list ap;
	size_t len = strlen(pcie_status);

	__va_start(ap, fmt);
	kvsnprintf(line, sizeof(line), fmt, ap);
	__va_end(ap);
	kprintf("pcie0: %s\n", line);
	if (len + strlen(line) + 2 < sizeof(pcie_status)) {
		strlcat(pcie_status, line, sizeof(pcie_status));
		strlcat(pcie_status, "\n", sizeof(pcie_status));
	}
	athp_dfly_crumb(line);		/* survives a reset (if_athp_dfly.c) */
}

/*
 * While Probe runs (its own thread), hold a risky step's status line on
 * screen for a second, so a hang shows which step it was.
 */
static thread_t probe_td;

static void
pace(void)
{
	if (curthread == probe_td)
		tsleep(&probe_td, 0, "pcipace", hz);
}

static inline uint32_t
rd(vm_paddr_t pa)
{
	return (*(volatile uint32_t *)PHYS_TO_DMAP(pa));
}

static inline void
wr(vm_paddr_t pa, uint32_t v)
{
	*(volatile uint32_t *)PHYS_TO_DMAP(pa) = v;
	__asm __volatile("dsb sy" : : : "memory");
}

static inline void
rmw(vm_paddr_t pa, uint32_t clear, uint32_t set)
{
	wr(pa, (rd(pa) & ~clear) | set);
}

static int
wait_bits(vm_paddr_t pa, uint32_t mask, uint32_t want, int us)
{
	while (us-- > 0) {
		if ((rd(pa) & mask) == want)
			return (0);
		DELAY(1);
	}
	return (ETIMEDOUT);
}

static void
phy_wr(uint32_t off, uint32_t v)
{
	wr(PHY + off, v);
}

/* a branch clock on: its enable bit, then CLK_OFF (bit 31) clear */
static int
branch_on(const char *name, vm_paddr_t cbcr)
{
	wr(cbcr, rd(cbcr) | 1U);
	if (wait_bits(cbcr, 1U << 31, 0, 500)) {
		st("clock %s stays off (0x%08x)", name, rd(cbcr));
		return (EIO);
	}
	return (0);
}

/* a GDSC on: clear SW_COLLAPSE, wait for PWR_ON (bit 31) */
static int
gdsc_on(const char *name, vm_paddr_t gdscr)
{
	/*
	 * As Linux's gdsc driver does at probe: no hardware trigger, no
	 * software override, so the domain cannot collapse on its own
	 * while idle (aggre0 did: the SMMU's registers then hung a read).
	 */
	wr(gdscr, rd(gdscr) & ~((1U << 1) | (1U << 2)));
	if (rd(gdscr) & (1U << 31))
		return (0);
	wr(gdscr, rd(gdscr) & ~1U);
	if (wait_bits(gdscr, 1U << 31, 1U << 31, 10000)) {
		st("power domain %s stays off (0x%08x)", name, rd(gdscr));
		return (EIO);
	}
	return (0);
}

/* a TLMM pin as a GPIO output at level v, drive 2 mA (or 16) */
static void
tlmm_out(int n, int v, int drive16)
{
	wr(TLMM_IO(n), v ? 2 : 0);
	/* FUNC_SEL 0 (bits 5:2), DRV (8:6), OE (9) */
	wr(TLMM_CFG(n), (rd(TLMM_CFG(n)) & ~0x3fcU) | ((drive16 ? 7U : 0U) << 6) |
	    (1U << 9));
	wr(TLMM_IO(n), v ? 2 : 0);
}

/* pci-msm.c's pcie_phy_init() for a 14 nm QMP PHY, phy_ver 3 */
static void
phy_common_init(void)
{
	phy_wr(PCIE_COM_POWER_DOWN_CONTROL, 0x01);
	phy_wr(COM_BIAS_EN_CLKBUFLR_EN, 0x1c);
	phy_wr(COM_CLK_ENABLE1, 0x10);
	phy_wr(COM_CLK_SELECT, 0x33);
	phy_wr(COM_CMN_CONFIG, 0x06);
	phy_wr(COM_LOCK_CMP_EN, 0x42);
	phy_wr(COM_VCO_TUNE_MAP, 0x00);
	phy_wr(COM_VCO_TUNE_TIMER1, 0xff);
	phy_wr(COM_VCO_TUNE_TIMER2, 0x1f);
	phy_wr(COM_HSCLK_SEL, 0x01);
	phy_wr(COM_SVS_MODE_CLK_SEL, 0x01);
	phy_wr(COM_CORE_CLK_EN, 0x00);
	phy_wr(COM_CORECLK_DIV, 0x0a);
	phy_wr(COM_BG_TIMER, 0x09);
	phy_wr(COM_DEC_START_MODE0, 0x82);
	phy_wr(COM_DIV_FRAC_START3_MODE0, 0x03);
	phy_wr(COM_DIV_FRAC_START2_MODE0, 0x55);
	phy_wr(COM_DIV_FRAC_START1_MODE0, 0x55);
	phy_wr(COM_LOCK_CMP3_MODE0, 0x00);
	phy_wr(COM_LOCK_CMP2_MODE0, 0x1a);
	phy_wr(COM_LOCK_CMP1_MODE0, 0x0a);
	phy_wr(COM_CLK_SELECT, 0x33);
	phy_wr(COM_SYS_CLK_CTRL, 0x02);
	phy_wr(COM_SYSCLK_BUF_ENABLE, 0x1f);
	phy_wr(COM_SYSCLK_EN_SEL, 0x04);
	phy_wr(COM_CP_CTRL_MODE0, 0x0b);
	phy_wr(COM_PLL_RCTRL_MODE0, 0x16);
	phy_wr(COM_PLL_CCTRL_MODE0, 0x28);
	phy_wr(COM_INTEGLOOP_GAIN1_MODE0, 0x00);
	phy_wr(COM_INTEGLOOP_GAIN0_MODE0, 0x80);
	phy_wr(COM_SSC_EN_CENTER, 0x01);
	phy_wr(COM_SSC_PER1, 0x31);
	phy_wr(COM_SSC_PER2, 0x01);
	phy_wr(COM_SSC_ADJ_PER1, 0x02);
	phy_wr(COM_SSC_ADJ_PER2, 0x00);
	phy_wr(COM_SSC_STEP_SIZE1, 0x2f);
	phy_wr(COM_SSC_STEP_SIZE2, 0x19);
	phy_wr(COM_RESCODE_DIV_NUM, 0x15);
	phy_wr(COM_BG_TRIM, 0x0f);
	phy_wr(COM_PLL_IVCO, 0x0f);
	phy_wr(COM_CLK_EP_DIV, 0x19);
	phy_wr(COM_CLK_ENABLE1, 0x10);
	/* phy_ver 3 */
	phy_wr(COM_HSCLK_SEL, 0x00);
	phy_wr(COM_RESCODE_DIV_NUM, 0x40);
	phy_wr(PCIE_COM_SW_RESET, 0x00);
	phy_wr(PCIE_COM_START_CONTROL, 0x03);
}

/* pci-msm.c's pcie_pcs_port_phy_init(), port 0 */
static void
phy_port_init(void)
{
	phy_wr(TX0 + 0x68, 0x45);	/* HIGHZ_TRANSCEIVEREN_BIAS_DRVR_EN */
	phy_wr(TX0 + 0x94, 0x06);	/* LANE_MODE */
	phy_wr(RX0 + 0x110, 0x1c);	/* SIGDET_ENABLES */
	phy_wr(RX0 + 0x118, 0x17);	/* SIGDET_LVL */
	phy_wr(RX0 + 0x0d8, 0x01);	/* RX_EQU_ADAPTOR_CNTRL2 */
	phy_wr(RX0 + 0x0dc, 0x00);	/* RX_EQU_ADAPTOR_CNTRL3 */
	phy_wr(RX0 + 0x0e0, 0xdb);	/* RX_EQU_ADAPTOR_CNTRL4 */
	phy_wr(RX0 + 0x120, 0x18);	/* RX_BAND */
	phy_wr(RX0 + 0x01c, 0x04);	/* UCDR_SO_GAIN */
	phy_wr(RX0 + 0x010, 0x04);	/* UCDR_SO_GAIN_HALF */
	phy_wr(PCS0 + 0x58, 0x4c);	/* RX_IDLE_DTCT_CNTRL */
	phy_wr(PCS0 + 0xa0, 0x00);	/* PWRUP_RESET_DLY_TIME_AUXCLK */
	phy_wr(PCS0 + 0xa4, 0x01);	/* LP_WAKEUP_DLY_TIME_AUXCLK */
	phy_wr(PCS0 + 0xa8, 0x05);	/* PLL_LOCK_CHK_DLY_TIME */
	phy_wr(RX0 + 0x048, 0x4b);	/* UCDR_SO_SATURATION_AND_ENABLE */
	phy_wr(RX0 + 0x11c, 0x14);	/* SIGDET_DEGLITCH_CNTRL */
	phy_wr(PCS0 + 0x54, 0x05);	/* ENDPOINT_REFCLK_DRIVE */
	phy_wr(PCS0 + 0x04, 0x02);	/* POWER_DOWN_CONTROL */
	phy_wr(PCS0 + 0x6c, 0x00);	/* POWER_STATE_CONFIG4 */
	phy_wr(PCS0 + 0x60, 0xa3);	/* POWER_STATE_CONFIG1 */
	/* phy_ver 3 */
	phy_wr(RX0 + 0x118, 0x19);	/* SIGDET_LVL */
	phy_wr(PCS0 + 0x28, 0x0e);	/* TXDEEMPH_M3P5DB_V0 */
	phy_wr(PCS0 + 0x04, 0x03);	/* POWER_DOWN_CONTROL */
	DELAY(11);
	phy_wr(PCS0 + 0x00, 0x00);	/* SW_RESET */
	phy_wr(PCS0 + 0x08, 0x0a);	/* START_CONTROL */
}

static void
iatu(int nr, uint32_t type, uint64_t host, uint32_t host_end, uint64_t target)
{
	wr(DBI + IATU_VIEWPORT, nr);
	wr(DBI + IATU_CTRL2, 0);
	wr(DBI + IATU_CTRL1, type);
	wr(DBI + IATU_LBAR, (uint32_t)host);
	wr(DBI + IATU_UBAR, (uint32_t)(host >> 32));
	wr(DBI + IATU_LAR, host_end);
	wr(DBI + IATU_LTAR, (uint32_t)target);
	wr(DBI + IATU_UTAR, (uint32_t)(target >> 32));
	wr(DBI + IATU_CTRL2, 1U << 31);
}

/* the endpoint's configuration space (bus 1, device 0, function 0) */
uint32_t
msm8996_pcie_cfg_read(int off)
{
	return (rd(CONF + off));
}

void
msm8996_pcie_cfg_write(int off, uint32_t v)
{
	wr(CONF + off, v);
}

static vm_paddr_t wlan_bar;
static uint32_t wlan_bar_mask;

vm_paddr_t
msm8996_pcie_bar0(void)
{
	return (wlan_bar);
}

/* power for the QCA6174, the cnss node's regulators */
static void
wlan_power(void)
{
	uint8_t v = 0;

	/* L30 1.8 V (the crystal), through the RPM */
	msm8996_rpm_write(0, RPM_LDOA, 30, RPM_KEY_UV, 1800000);
	msm8996_rpm_write(0, RPM_LDOA, 30, RPM_KEY_SWEN, 1);
	/*
	 * PM8994 GPIO 9 (the 3.3 V switch, set to constant high by the
	 * board), then GPIO 8, WLAN_EN: digital output, VIN2, driven high
	 * (MODE_CTL: mode 1 in bits 6:4, source 0, invert 1)
	 */
	msm8996_spmi_read(PM8994_SID, PM8994_GPIO(8) + 0x40, &v);
	st("pm8994 gpio8 (WLAN_EN) was 0x%02x", v);

	/*
	 * PM8994 GPIO 18, "Rome Sleep Clock" in Xiaomi's board file: the
	 * PMIC's 32 kHz sleep clock out to the chip (digital output, VIN2,
	 * source "function 2", medium drive).  Without it the QCA6174's
	 * always-on domain never runs and the SoC cannot be woken.
	 * MODE_CTL: mode 1 << 4 | source 3 << 1; DIG_OUT_CTL: CMOS, medium.
	 */
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(18) + 0x41, 2);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(18) + 0x45, 0x02);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(18) + 0x40, 0x16);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(18) + 0x46, 0x80);
	msm8996_spmi_read(PM8994_SID, PM8994_GPIO(18) + 0x40, &v);
	st("pm8994 gpio18 (sleep clock) mode 0x%02x", v);
	DELAY(1000);

	msm8996_spmi_read(PM8994_SID, PM8994_GPIO(9) + 0x40, &v);
	st("pm8994 gpio9 (3.3 V) mode 0x%02x", v);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(9) + 0x40, 0x11);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(9) + 0x46, 0x80);
	DELAY(4000);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(8) + 0x41, 2);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(8) + 0x40, 0x11);
	msm8996_spmi_write(PM8994_SID, PM8994_GPIO(8) + 0x46, 0x80);
	msm8996_spmi_read(PM8994_SID, PM8994_GPIO(8) + 0x40, &v);
	st("pm8994 gpio8 (WLAN_EN) mode 0x%02x", v);
	/* the bootstrap pin, high, 16 mA */
	tlmm_out(BOOTSTRAP_GPIO, 1, 1);
	DELAY(100000);		/* crystal and always-on domain settle */
}

static int
pcie0_up(void)
{
	uint32_t v;
	int i;

	pcie_status[0] = 0;
	if (!msm8996_rpm_ready()) {
		st("the RPM is not up; try again later");
		return (EAGAIN);
	}

	/* hold the endpoint in reset (PERST# low) */
	tlmm_out(PERST_GPIO, 0, 0);
	DELAY(1000);
	wlan_power();

	/* the controller's supplies: L12 1.8 V, L28 0.925 V */
	msm8996_rpm_write(0, RPM_LDOA, 12, RPM_KEY_UV, 1800000);
	msm8996_rpm_write(0, RPM_LDOA, 12, RPM_KEY_SWEN, 1);
	msm8996_rpm_write(0, RPM_LDOA, 28, RPM_KEY_UV, 925000);
	msm8996_rpm_write(0, RPM_LDOA, 28, RPM_KEY_SWEN, 1);
	/* the PHY's reference clock: the RPM's ln_bb_clk */
	msm8996_rpm_write(0, RPM_MISC_CLK, RPM_LN_BB_CLK, RPM_KEY_ENAB, 1);

	/* power domains and clocks (msm_pcie_clk_init) */
	if (gdsc_on("aggre0_noc", GCC_AGGRE0_NOC_GDSCR) ||
	    gdsc_on("pcie_0", GCC_PCIE_0_GDSCR))
		return (EIO);
	st("power domains: aggre0 %08x (status %08x) pcie_0 %08x",
	    rd(GCC_AGGRE0_NOC_GDSCR), rd(GCC + 0x81028), rd(GCC_PCIE_0_GDSCR));
	/* aux clock source: XO / 19 through M/N, 1.0105 MHz */
	wr(GCC_PCIE_AUX_CMD_RCGR + 0x08, 1);			/* M */
	wr(GCC_PCIE_AUX_CMD_RCGR + 0x0c, ~(19U - 1U) & 0xffff);	/* N */
	wr(GCC_PCIE_AUX_CMD_RCGR + 0x10, ~19U & 0xffff);		/* D */
	wr(GCC_PCIE_AUX_CMD_RCGR + 0x04, (2U << 12) | (0U << 8));	/* dual edge */
	wr(GCC_PCIE_AUX_CMD_RCGR, rd(GCC_PCIE_AUX_CMD_RCGR) | 1U);
	(void)wait_bits(GCC_PCIE_AUX_CMD_RCGR, 1U, 0, 500);
	if (branch_on("pcie_0_aux", GCC_PCIE_0_AUX_CBCR) ||
	    branch_on("pcie_0_cfg_ahb", GCC_PCIE_0_CFG_AHB_CBCR) ||
	    branch_on("pcie_0_mstr_axi", GCC_PCIE_0_MSTR_AXI_CBCR) ||
	    branch_on("pcie_0_slv_axi", GCC_PCIE_0_SLV_AXI_CBCR) ||
	    branch_on("smmu_aggre0_axi", GCC_SMMU_AGGRE0_AXI_CBCR) ||
	    branch_on("smmu_aggre0_ahb", GCC_SMMU_AGGRE0_AHB_CBCR) ||
	    branch_on("pcie_phy_cfg_ahb", GCC_PCIE_PHY_CFG_AHB_CBCR) ||
	    branch_on("pcie_phy_aux", GCC_PCIE_PHY_AUX_CBCR))
		return (EIO);
	wr(GCC_PCIE_CLKREF_EN, rd(GCC_PCIE_CLKREF_EN) | 1U);
	/* resets released */
	rmw(GCC_PCIE_PHY_BCR, 1U, 0);
	rmw(GCC_PCIE_PHY_COM_BCR, 1U, 0);
	rmw(GCC_PCIE_PHY_NOCSR_COM_PHY_BCR, 1U, 0);
	rmw(GCC_PCIE_0_PHY_BCR, 1U, 0);
	DELAY(10);
	/*
	 * The path from PCIe to RAM: a0noc -> snoc -> DDR.  a0noc's bus
	 * clock is gcc_aggre0_snoc_axi_clk (msm8996-bus.dtsi, fab-a0noc),
	 * which nothing turned on: the chip's DMA reads never completed.
	 */
	st("a0noc clocks: enabling snoc_axi (was %08x) cnoc_ahb (was %08x)",
	    rd(GCC_AGGRE0_SNOC_AXI_CBCR), rd(GCC_AGGRE0_CNOC_AHB_CBCR));
	pace();
	/*
	 * Mainline Linux keeps every aggre0 clock on (CLK_IS_CRITICAL in
	 * gcc-msm8996): the NoC's bus and config clocks, its MPU (access
	 * control) config clock, the QoS generator's reference, and the
	 * SMMU's.  Offsets from Xiaomi's msm-clocks-hwio-8996.h.
	 */
	wr(GCC_AGGRE0_SNOC_AXI_CBCR, rd(GCC_AGGRE0_SNOC_AXI_CBCR) | 1U);
	wr(GCC_AGGRE0_CNOC_AHB_CBCR, rd(GCC_AGGRE0_CNOC_AHB_CBCR) | 1U);
	wr(GCC + 0x85000, rd(GCC + 0x85000) | 1U);	/* aggre0_noc_mpu_cfg_ahb */
	wr(GCC + 0x8101c, rd(GCC + 0x8101c) | 1U);	/* aggre0_noc_qosgen_extref */
	DELAY(100);
	st("a0noc clocks on: snoc_axi %08x cnoc_ahb %08x mpu_cfg %08x",
	    rd(GCC_AGGRE0_SNOC_AXI_CBCR), rd(GCC_AGGRE0_CNOC_AHB_CBCR),
	    rd(GCC + 0x85000));
	pace();
	st("clocks on; parf version reads 0x%08x", rd(PARF + 0x00));
	smmu_bypass();

	/* the controller (msm_pcie_enable) */
	rmw(PARF + PARF_PHY_CTRL, 1U, 0);
	wr(PARF + PARF_DBI_BASE_ADDR, 0);
	wr(PARF + PARF_SYS_CTRL, 0x365e);
	rmw(PARF + PARF_MHI_CLOCK_RESET_CTRL, 0, 1U << 4);
	wr(PARF + PARF_SLV_ADDR_SPACE_SIZE, 0x01000000);	/* 16 MB */
	/* mainline qcom_pcie_post_init_2_3_2: AXI master write-address halt */
	rmw(PARF + 0x1a8, 0, 1U << 31);

	/* the PHY, then its pipe clock */
	phy_common_init();
	DELAY(1000);
	if (branch_on("pcie_0_pipe", GCC_PCIE_0_PIPE_CBCR))
		st("(the pipe clock comes from the PHY; continuing)");
	for (i = 0; i < 10; ++i) {
		if (rd(PHY + PCIE_COM_PCS_READY_STATUS) & 1)
			break;
		DELAY(1000);
	}
	if (!(rd(PHY + PCIE_COM_PCS_READY_STATUS) & 1)) {
		st("PHY not ready (pcs ready status 0x%08x)",
		    rd(PHY + PCIE_COM_PCS_READY_STATUS));
		return (EIO);
	}
	st("PHY ready after %d ms", i);
	phy_port_init();
	DELAY(10000);			/* qcom,ep-latency = <10> */

	/* release PERST#, then train the link */
	tlmm_out(PERST_GPIO, 1, 0);
	DELAY(1000);
	rmw(DBI + DBI_DEVICE_CONTROL_STATUS, 0x7000, 5U << 12);	/* TLP rd 4K */
	rmw(PARF + PARF_LTSSM, 0, 1U << 8);
	for (i = 0; i < 200; ++i) {		/* up to 1 s */
		DELAY(5000);
		v = rd(ELBI + ELBI_SYS_STTS);
		if ((v & XMLH_LINK_UP) && (rd(DBI + DBI_LINK_STATUS) & (1U << 29)))
			break;
	}
	if (!(v & XMLH_LINK_UP)) {
		st("no link: ltssm state 0x%x (elbi 0x%08x)", (v >> 12) & 0x3f,
		    v);
		tlmm_out(PERST_GPIO, 0, 0);
		return (EIO);
	}
	st("link up after %d ms: gen %u x%u", (i + 1) * 5,
	    (rd(DBI + DBI_LINK_STATUS) >> 16) & 0xf,
	    (rd(DBI + DBI_LINK_STATUS) >> 20) & 0x3f);

	/* msm_pcie_config_controller */
	iatu(0, IATU_TYPE_CFG0, CONF, CONF + 0xfff, 0x01000000);
	rmw(DBI + DBI_ACK_F_ASPM_CTRL, 0, 1U << 15);
	wr(DBI + DBI_AUX_CLK_FREQ, 0x01);
	wr(PARF + PARF_BDF_TRANSLATE_CFG, 0);
	wr(PARF + PARF_SID_OFFSET, 0);
	/* dw_pcie_setup_rc: the root port claims no inbound address range */
	wr(DBI + 0x10, 0);	/* PCI_BASE_ADDRESS_0 */
	wr(DBI + 0x14, 0);	/* PCI_BASE_ADDRESS_1 */
	/* the root port: bus 0 -> 1, memory window over the BAR space */
	wr(DBI + DBI_BUSNUMBERS, 0x00010100);
	wr(DBI + DBI_MEMORY_BASE_LIMIT,
	    ((uint32_t)(BARS + BARS_SIZE - 1) & 0xfff00000U) |
	    ((uint32_t)BARS >> 16));
	rmw(DBI + DBI_COMMAND_STATUS, 0, 0x6);	/* memory + bus master */

	v = msm8996_pcie_cfg_read(0);
	wlan_id = (int)((v & 0xffff) << 16 | v >> 16);
	st("root complex 0x%08x; endpoint vendor 0x%04x device 0x%04x%s",
	    rd(DBI), v & 0xffff, v >> 16,
	    v == 0x003e168c ? " (QCA6174)" : "");
	if (v == 0xffffffffU || v == 0)
		return (ENXIO);

	/*
	 * BAR0 inside the BAR window, through iATU region 1.  A BAR must be
	 * aligned to its size: QCA6174's is 2 MB, and at 0x0c300000 the chip
	 * quietly rounded it down to 0x0c200000, so every access landed
	 * 1 MB off (its always-on registers looked dead).
	 */
	msm8996_pcie_cfg_write(0x10, 0xffffffffU);
	v = msm8996_pcie_cfg_read(0x10);
	st("BAR0 size mask 0x%08x", v);
	wlan_bar_mask = v;
	{
		vm_paddr_t size = (vm_paddr_t)(~(v & ~0xfU)) + 1;
		vm_paddr_t bar = roundup2(BARS, size);

		if (size == 0 || bar + size > BARS + BARS_SIZE) {
			st("BAR0 of %lu bytes does not fit the window",
			    (unsigned long)size);
			return (ENXIO);
		}
		wlan_bar = bar;
	}
	msm8996_pcie_cfg_write(0x10, (uint32_t)wlan_bar);
	if ((v & 0x6) == 0x4)
		msm8996_pcie_cfg_write(0x14, 0);
	iatu(1, IATU_TYPE_MEM, BARS, (uint32_t)(BARS + BARS_SIZE - 1), BARS);
	msm8996_pcie_cfg_write(0x04, msm8996_pcie_cfg_read(0x04) | 0x6);
	/* QCA6174: SOC_CHIP_ID at RTC_SOC_BASE 0x800 + 0xf0 */
	st("chip id 0x%08x (BAR0 at 0x%08lx, reads back 0x%08x)",
	    rd(wlan_bar + 0x8f0), (unsigned long)wlan_bar,
	    msm8996_pcie_cfg_read(0x10));
	return (0);
}

/*
 * The chip's DMA goes through anoc0's SMMU.  Nothing here uses IOMMU
 * translation, so put it in bypass (sCR0.CLIENTPD) and stop unmatched
 * streams faulting (USFCFG).  Linux sets up a translation context per
 * device instead (qcom,smmu-exist).
 */
static void
smmu_bypass(void)
{
	uint32_t v;

	st("smmu: reading (scr0 at %#lx)", (unsigned long)ANOC0_SMMU);
	pace();
	v = rd(ANOC0_SMMU + SMMU_SCR0);
	st("smmu: scr0 0x%08x idr0 0x%08x gfsr 0x%08x", v,
	    rd(ANOC0_SMMU + SMMU_SIDR0), rd(ANOC0_SMMU + SMMU_SGFSR));
	wr(ANOC0_SMMU + SMMU_SCR0, (v | 1U) & ~(1U << 10));
	wr(ANOC0_SMMU + SMMU_SGFSR, rd(ANOC0_SMMU + SMMU_SGFSR));
	st("smmu: bypass, scr0 now 0x%08x", rd(ANOC0_SMMU + SMMU_SCR0));
	/*
	 * No catch-all entry here: an SMR with a full mask is dropped by
	 * this SoC's SMMUs and the next access resets the phone (seen on
	 * the GPU's).  msm8996_pcie_smmu_fix() adds an exact one for the
	 * chip's stream once a stalled transfer names it.
	 */
}

/*
 * The secure side keeps sCR0 (no global bypass, unmatched streams
 * fault), so the chip's stream is caught by a stream-match entry set up
 * by the boot firmware, pointing at a context bank with nothing mapped.
 * After a stalled transfer: find the context bank that faulted, read
 * the faulting stream id (CBFRSYNRA), and turn every matching SMR's
 * S2CR to bypass.  An unidentified-stream fault (sGFSR.USF) instead
 * gets a new bypass SMR for exactly its id.  Logs what it saw and did;
 * returns the number of entries changed.
 */
#define	SMMU_SIDR1		0x024
#define	SMMU_SMR(n)		(0x800 + 4 * (n))
#define	SMMU_S2CR(n)		(0xc00 + 4 * (n))
#define	SMMU_CB_FSR		0x058
#define	SMMU_CB_FAR		0x060
#define	S2CR_BYPASS		(1U << 16)

int
msm8996_pcie_smmu_fix(void)
{
	uint32_t idr0 = rd(ANOC0_SMMU + SMMU_SIDR0);
	uint32_t idr1 = rd(ANOC0_SMMU + SMMU_SIDR1);
	uint32_t pg = (idr1 & (1U << 31)) ? 0x10000 : 0x1000;
	uint32_t npage = 1U << (((idr1 >> 28) & 7) + 1);
	uint32_t nsmr = idr0 & 0xff, ncb = idr1 & 0xff, gfsr, sid, smr, s2cr;
	vm_paddr_t cb0 = ANOC0_SMMU + (vm_paddr_t)npage * pg;
	int n, i, changed = 0, shown = 0;
	char line[160];
	int len = 0;

	/* the stream-match table as found */
	for (i = 0; i < (int)nsmr && i < 128; ++i) {
		smr = rd(ANOC0_SMMU + SMMU_SMR(i));
		if (!(smr & (1U << 31)))
			continue;
		len += ksnprintf(line + len, sizeof(line) - len, "%d:%x/%x>%x ",
		    i, smr & 0x7fff, (smr >> 16) & 0x7fff,
		    rd(ANOC0_SMMU + SMMU_S2CR(i)));
		if (len > 110) {
			kprintf("smmu smr %s\n", line);
			len = 0;
			shown++;
		}
	}
	if (len)
		kprintf("smmu smr %s\n", line);
	kprintf("smmu: %u smr, %u cb, cb0 at %#lx, gfsr %08x\n", nsmr, ncb,
	    (unsigned long)cb0, rd(ANOC0_SMMU + SMMU_SGFSR));

	/* context-bank faults: whose stream? */
	for (n = 0; n < (int)ncb && n < 64; ++n) {
		vm_paddr_t cb = cb0 + (vm_paddr_t)n * pg;
		uint32_t fsr = rd(cb + SMMU_CB_FSR);

		if (fsr == 0)
			continue;
		sid = rd(ANOC0_SMMU + pg + 0x400 + 4 * n) & 0xffff;
		kprintf("smmu: cb%d fsr %08x far %08x%08x sid %#x\n", n, fsr,
		    rd(cb + SMMU_CB_FAR + 4), rd(cb + SMMU_CB_FAR), sid);
		for (i = 0; i < (int)nsmr && i < 128; ++i) {
			smr = rd(ANOC0_SMMU + SMMU_SMR(i));
			if (!(smr & (1U << 31)))
				continue;
			if (((smr ^ sid) & ~((smr >> 16) & 0x7fff) & 0x7fff) != 0)
				continue;
			s2cr = rd(ANOC0_SMMU + SMMU_S2CR(i));
			if ((s2cr & (3U << 16)) == S2CR_BYPASS)
				continue;
			wr(ANOC0_SMMU + SMMU_S2CR(i), S2CR_BYPASS);
			kprintf("smmu: smr %d (sid %#x) %08x -> bypass, now %08x\n",
			    i, sid, s2cr, rd(ANOC0_SMMU + SMMU_S2CR(i)));
			changed++;
		}
		wr(cb + SMMU_CB_FSR, fsr);	/* clear */
	}

	/* an unidentified stream: give it its own bypass entry */
	gfsr = rd(ANOC0_SMMU + SMMU_SGFSR);
	if (gfsr & 2) {
		sid = rd(ANOC0_SMMU + SMMU_SGFSYNR1) & 0x7fff;
		for (i = 0; i < (int)nsmr && i < 128; ++i)
			if (!(rd(ANOC0_SMMU + SMMU_SMR(i)) & (1U << 31)))
				break;
		if (i < (int)nsmr) {
			wr(ANOC0_SMMU + SMMU_S2CR(i), S2CR_BYPASS);
			wr(ANOC0_SMMU + SMMU_SMR(i), (1U << 31) | sid);
			__asm __volatile("dsb sy" : : : "memory");
			s2cr = rd(ANOC0_SMMU + SMMU_S2CR(i));
			if ((s2cr & (3U << 16)) != S2CR_BYPASS && ncb > 0) {
				/*
				 * Bypass refused (reads back as fault), as on
				 * the GPU's SMMU: route the stream through the
				 * last context bank with translation off
				 * (SCTLR 0; CBAR stage 1, stage 2 bypass), the
				 * way Linux's qcom bypass quirk does.
				 */
				int cb = ncb - 1;

				wr(cb0 + (vm_paddr_t)cb * pg, 0);	/* SCTLR */
				wr(ANOC0_SMMU + pg + 4 * cb, 1U << 16);	/* CBAR */
				wr(ANOC0_SMMU + SMMU_S2CR(i), (uint32_t)cb);
				__asm __volatile("dsb sy" : : : "memory");
				kprintf("smmu: bypass refused (%08x); sid %#x via "
				    "cb%d, translation off: s2cr %08x cbar %08x\n",
				    s2cr, sid, cb, rd(ANOC0_SMMU + SMMU_S2CR(i)),
				    rd(ANOC0_SMMU + pg + 4 * cb));
			}
			kprintf("smmu: unmatched sid %#x -> smr %d (%08x) s2cr "
			    "%08x\n", sid, i, rd(ANOC0_SMMU + SMMU_SMR(i)),
			    rd(ANOC0_SMMU + SMMU_S2CR(i)));
			changed++;
		}
		wr(ANOC0_SMMU + SMMU_SGFSR, gfsr);
	}
	(void)shown;
	return (changed);
}

/* the SMMU's global fault state, for the driver's diagnostics */
void
msm8996_pcie_smmu_diag(char *buf, size_t len)
{
	ksnprintf(buf, len, "smmu scr0 %08x gfsr %08x gfsynr1 %08x",
	    rd(ANOC0_SMMU + SMMU_SCR0), rd(ANOC0_SMMU + SMMU_SGFSR),
	    rd(ANOC0_SMMU + SMMU_SGFSYNR1));
}

/*
 * One line on the Wi-Fi chip's supplies, straight from the PMIC: L30
 * (the crystal's 1.8 V; PM8994 regulators sit on SID 1, LDO n at
 * 0x4000 + (n - 1) * 0x100) and GPIOs 8 (WLAN_EN), 9 (3.3 V), 18 (sleep
 * clock): their status and mode registers.
 */
void
msm8996_wlan_power_diag(char *buf, size_t len)
{
	uint8_t l30s = 0, l30v = 0, l30e = 0, g[3][2];
	static const int gpios[3] = { 8, 9, 18 };
	int i;

	msm8996_spmi_read(1, 0x5d00 + 0x08, &l30s);
	msm8996_spmi_read(1, 0x5d00 + 0x41, &l30v);
	msm8996_spmi_read(1, 0x5d00 + 0x46, &l30e);
	for (i = 0; i < 3; ++i) {
		g[i][0] = g[i][1] = 0;
		msm8996_spmi_read(PM8994_SID, PM8994_GPIO(gpios[i]) + 0x08,
		    &g[i][0]);
		msm8996_spmi_read(PM8994_SID, PM8994_GPIO(gpios[i]) + 0x40,
		    &g[i][1]);
	}
	ksnprintf(buf, len, "L30 %02x/%02x/%02x g8 %02x/%02x g9 %02x/%02x "
	    "g18 %02x/%02x bar %08x/%08x cmd %04x", l30s, l30v, l30e,
	    g[0][0], g[0][1], g[1][0], g[1][1], g[2][0], g[2][1],
	    wlan_bar_mask, msm8996_pcie_cfg_read(0x10),
	    msm8996_pcie_cfg_read(0x04) & 0xffff);
}

/* Is the link trained?  Reads only root-complex registers: always safe. */
int
msm8996_pcie_link_ok(void)
{
	return ((rd(ELBI + ELBI_SYS_STTS) & XMLH_LINK_UP) &&
	    (rd(DBI + DBI_LINK_STATUS) & (1U << 29)));
}

/*
 * After the endpoint reset itself (ath10k's cold reset takes its PCIe
 * core down too): wait up to ms for the link to train again, then put
 * back BAR0 and memory/bus-master enable if the reset cleared them.
 * Until this returns 0, nothing may read through the BAR: a read over a
 * dead link stalls the whole SoC.
 */
int
msm8996_pcie_relink(int ms)
{
	uint32_t v;
	int i;

	for (i = 0; i < ms && !msm8996_pcie_link_ok(); ++i)
		DELAY(1000);
	if (!msm8996_pcie_link_ok()) {
		st("relink: no link after %d ms (ltssm 0x%x)", ms,
		    (rd(ELBI + ELBI_SYS_STTS) >> 12) & 0x3f);
		return (EIO);
	}
	/* configuration requests may be retried for a while */
	v = msm8996_pcie_cfg_read(0);
	for (i = 0; i < 100 && (v == 0xffffffffU || v == 0); ++i) {
		DELAY(1000);
		v = msm8996_pcie_cfg_read(0);
	}
	if ((msm8996_pcie_cfg_read(0x10) & ~0xfU) != (uint32_t)wlan_bar) {
		msm8996_pcie_cfg_write(0x10, (uint32_t)wlan_bar);
		st("relink: BAR0 restored");
	}
	if ((msm8996_pcie_cfg_read(0x04) & 0x6) != 0x6)
		msm8996_pcie_cfg_write(0x04, msm8996_pcie_cfg_read(0x04) | 0x6);
	return (0);
}

int
msm8996_pcie_up(void)
{
	int error;

	lockmgr(&pcie_lock, LK_EXCLUSIVE);
	if (pcie_done) {
		lockmgr(&pcie_lock, LK_RELEASE);
		return (wlan_bar ? 0 : EIO);
	}
	error = pcie0_up();
	pcie_done = (error == 0);	/* a failed bring-up may be retried */
	lockmgr(&pcie_lock, LK_RELEASE);
	return (error);
}

static void
pcie_probe_thread(void *arg)
{
	(void)msm8996_pcie_up();
	probe_td = NULL;
	kthread_exit();
}

static int
sysctl_pcie_up(SYSCTL_HANDLER_ARGS)
{
	int v = pcie_done, error;

	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error || req->newptr == NULL)
		return (error);
	/*
	 * Bring-up takes seconds and may stall the SoC: run it in its own
	 * thread so the caller (the shell's Probe button) gets control back
	 * and keeps showing hw.mi5.pcie as it fills in.
	 */
	if (v && probe_td == NULL && !pcie_done)
		kthread_create(pcie_probe_thread, NULL, &probe_td, "pcie probe");
	return (0);
}

SYSCTL_PROC(_hw_mi5, OID_AUTO, pcie_up, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_pcie_up, "I", "bring PCIe RC0 (the Wi-Fi chip) up");
