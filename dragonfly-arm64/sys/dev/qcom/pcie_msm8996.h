/* SPDX-License-Identifier: GPL-2.0-only */
/* MSM8996 PCIe RC0, the Wi-Fi chip's link; see pcie_msm8996.c. */
#ifndef _DEV_QCOM_PCIE_MSM8996_H_
#define	_DEV_QCOM_PCIE_MSM8996_H_

int		msm8996_pcie_up(void);		/* 0 once the endpoint answers */
uint32_t	msm8996_pcie_cfg_read(int off);	/* bus 1, dev 0, fn 0 */
void		msm8996_pcie_cfg_write(int off, uint32_t v);
vm_paddr_t	msm8996_pcie_bar0(void);	/* physical, 0 if none */
int		msm8996_pcie_link_ok(void);	/* RC side only, always safe */
int		msm8996_pcie_relink(int ms);	/* after an endpoint reset */

#endif
