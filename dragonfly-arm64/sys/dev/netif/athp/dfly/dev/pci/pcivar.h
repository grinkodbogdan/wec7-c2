/* athp on DragonFly/Mi 5: no PCI bus, the one endpoint behind MSM8996 RC0. */
#ifndef _ATHP_DFLY_PCIVAR_H_
#define	_ATHP_DFLY_PCIVAR_H_
#include <bus/pci/pcireg.h>
uint32_t	athp_dfly_cfg_read(int reg, int width);
void		athp_dfly_cfg_write(int reg, uint32_t v, int width);
#define	pci_read_config(dev, reg, w)	athp_dfly_cfg_read((reg), (w))
#define	pci_write_config(dev, reg, v, w) athp_dfly_cfg_write((reg), (v), (w))
#define	pci_get_vendor(dev)	(athp_dfly_cfg_read(0, 2))
#define	pci_get_device(dev)	(athp_dfly_cfg_read(2, 2))
#define	pci_get_subvendor(dev)	(athp_dfly_cfg_read(0x2c, 2))
#define	pci_get_subdevice(dev)	(athp_dfly_cfg_read(0x2e, 2))
#define	pci_get_revid(dev)	(athp_dfly_cfg_read(8, 1))
#define	pci_enable_busmaster(dev) athp_dfly_cfg_write(4, athp_dfly_cfg_read(4, 2) | 6, 2)
#define	pci_msi_count(dev)	0
#define	pci_msix_count(dev)	0
#define	pci_alloc_msi(dev, n)	ENXIO
#define	pci_release_msi(dev)	0
int	athp_dfly_find_cap(int cap, int *offp);
#define	pci_find_cap(dev, c, r)	athp_dfly_find_cap((c), (r))
#define	pci_get_drvdata(dev)	device_get_softc(dev)
#endif
