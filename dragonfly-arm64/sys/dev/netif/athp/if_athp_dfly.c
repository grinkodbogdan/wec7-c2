/*
 * athp on DragonFly/arm64, Xiaomi Mi 5: what if_athp_pci.c gets from a
 * PCI bus on FreeBSD.  The QCA6174 is the one endpoint behind MSM8996
 * PCIe RC0 (dev/qcom/pcie_msm8996.c), so config space goes straight to
 * that root complex, the firmware is linked into the kernel, and the
 * device is created on demand:
 *
 *	sysctl hw.mi5.wlan_attach=1
 *
 * Status, for the shell: hw.mi5.wlan (a one-line string).
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/bus.h>
#include <sys/mbuf.h>
#include <sys/firmware.h>
#include <sys/sysctl.h>
#include <machine/stdarg.h>
#include <sys/kthread.h>

#include <dev/pci/pcivar.h>
#include <dev/qcom/pcie_msm8996.h>

/* ---- config space ---- */
uint32_t
athp_dfly_cfg_read(int reg, int width)
{
	uint32_t v;

	v = msm8996_pcie_cfg_read(reg & ~3) >> ((reg & 3) * 8);
	if (width == 1)
		v &= 0xff;
	else if (width == 2)
		v &= 0xffff;
	return (v);
}

void
athp_dfly_cfg_write(int reg, uint32_t val, int width)
{
	uint32_t mask, v;
	int sh;

	if (width == 4) {
		msm8996_pcie_cfg_write(reg & ~3, val);
		return;
	}
	sh = (reg & 3) * 8;
	mask = (width == 1 ? 0xffU : 0xffffU) << sh;
	v = msm8996_pcie_cfg_read(reg & ~3);
	v = (v & ~mask) | ((val << sh) & mask);
	msm8996_pcie_cfg_write(reg & ~3, v);
}

int
athp_dfly_find_cap(int cap, int *offp)
{
	int ptr, n;

	if ((athp_dfly_cfg_read(PCIR_STATUS, 2) & PCIM_STATUS_CAPPRESENT) == 0)
		return (ENXIO);
	ptr = athp_dfly_cfg_read(PCIR_CAP_PTR, 1) & ~3;
	for (n = 0; ptr != 0 && n < 48; n++) {
		if (athp_dfly_cfg_read(ptr + PCICAP_ID, 1) == cap) {
			*offp = ptr;
			return (0);
		}
		ptr = athp_dfly_cfg_read(ptr + PCICAP_NEXTPTR, 1) & ~3;
	}
	return (ENOENT);
}

/* ---- small API gaps ---- */
struct mbuf *
athp_dfly_m_getm2(int len, int how, int type, int flags)
{
	if (len > MJUMPAGESIZE)
		return (m_getjcl(how, type, flags, MJUM9BYTES));
	return (m_getl(len, how, type, flags, NULL));
}

struct ieee80211_node;
void
athp_dfly_seqno_assign(struct ieee80211_node *ni, struct mbuf *m)
{
	/* only raw mode needs it; native wifi mode lets net80211 number */
}

struct ath10k;
int
athp_ioctl_setup(struct ath10k *ar)
{
	return (0);	/* the debug ioctl device is left out */
}

void
athp_ioctl_teardown(struct ath10k *ar)
{
}

/*
 * ---- firmware ----
 * build.sh links the linux-firmware QCA6174 hw3.0 files in with
 * llvm-objcopy -I binary; weak, so a kernel without them still links.
 */
#define	FW_SYM(n)							\
	extern const char _binary_##n##_start[] __attribute__((weak));	\
	extern const char _binary_##n##_end[] __attribute__((weak))
FW_SYM(athp_fw4_bin);
FW_SYM(athp_board_bin);
FW_SYM(athp_board2_bin);

static const struct {
	const char	*name;
	const char	*start;
	const char	*end;
} athp_fw[] = {
	{ "athp_QCA6174_hw3.0_firmware-4.bin",
	    _binary_athp_fw4_bin_start, _binary_athp_fw4_bin_end },
	{ "athp_QCA6174_hw3.0_board.bin",
	    _binary_athp_board_bin_start, _binary_athp_board_bin_end },
	{ "athp_QCA6174_hw3.0_board-2.bin",
	    _binary_athp_board2_bin_start, _binary_athp_board2_bin_end },
};

static int
athp_dfly_fw_register(void)
{
	static int done;
	int i, n = 0;

	if (done)
		return (done);
	for (i = 0; i < (int)nitems(athp_fw); i++) {
		if (athp_fw[i].start == NULL || athp_fw[i].end <= athp_fw[i].start)
			continue;
		if (firmware_register(athp_fw[i].name, athp_fw[i].start,
		    athp_fw[i].end - athp_fw[i].start, 0, NULL) != NULL)
			n++;
	}
	done = n;
	return (n);
}

/* ---- attach on demand ---- */
volatile int athp_dfly_in_reset;
uint32_t athp_dfly_last_ind;
static char wlan_status[96] = "Not started";
static device_t athp_dev;
static thread_t athp_attach_td;	/* paces its steps for the UI */

SYSCTL_DECL(_hw_mi5);
SYSCTL_STRING(_hw_mi5, OID_AUTO, wlan, CTLFLAG_RD, wlan_status, 0,
    "Wi-Fi driver state");

static int
athp_dfly_attach(void)
{
	device_t dev;
	int error, nfw;

	if (athp_dev != NULL)
		return (0);
	if (msm8996_pcie_up() != 0) {
		ksnprintf(wlan_status, sizeof(wlan_status), "PCIe link down");
		return (EIO);
	}
	nfw = athp_dfly_fw_register();
	if (nfw == 0) {
		ksnprintf(wlan_status, sizeof(wlan_status),
		    "no firmware in this kernel");
		return (ENOENT);
	}
	ksnprintf(wlan_status, sizeof(wlan_status), "attaching (%04x:%04x)",
	    athp_dfly_cfg_read(PCIR_VENDOR, 2),
	    athp_dfly_cfg_read(PCIR_DEVICE, 2));
	dev = device_add_child(root_bus, "athp", 0);
	if (dev == NULL)
		return (ENXIO);
	error = device_probe_and_attach(dev);
	if (error != 0) {
		char last[sizeof(wlan_status)];

		char sm[80];

		msm8996_pcie_smmu_diag(sm, sizeof(sm));
		kprintf("athp diag: after failure: %s\n", sm);
		/* keep the step it failed at */
		strlcpy(last, wlan_status, sizeof(last));
		ksnprintf(wlan_status, sizeof(wlan_status),
		    "fail %d: %s", error, last);
		device_delete_child(root_bus, dev);
		return (error);
	}
	athp_dev = dev;
	return (0);
}

/* the same without the pause or the console line: for progress in loops */
void
athp_dfly_note(const char *fmt, ...)
{
	__va_list ap;

	__va_start(ap, fmt);
	kvsnprintf(wlan_status, sizeof(wlan_status), fmt, ap);
	__va_end(ap);
}

/* if_athp_core.c reports progress here */
void
athp_dfly_status(const char *fmt, ...)
{
	__va_list ap;

	__va_start(ap, fmt);
	kvsnprintf(wlan_status, sizeof(wlan_status), fmt, ap);
	__va_end(ap);
	kprintf("athp: %s\n", wlan_status);
	/*
	 * While attaching, hold each step on screen for longer than the
	 * shell's one-second refresh, so a hang shows where it happened.
	 */
	if (curthread == athp_attach_td)
		tsleep(&athp_attach_td, 0, "athpstep", hz * 3 / 2);
}

/*
 * Attaching adds sysctl nodes, which needs the sysctl lock exclusively;
 * a sysctl handler already holds it shared.  So the handler only starts
 * a thread, and the attach runs there.
 */
static volatile int athp_attaching;

static void
athp_dfly_attach_thread(void *arg)
{
	athp_attach_td = curthread;
	athp_dfly_attach();
	athp_attach_td = NULL;
	athp_attaching = 0;
	kthread_exit();
}

static int
sysctl_wlan_attach(SYSCTL_HANDLER_ARGS)
{
	int error, v;

	v = (athp_dev != NULL);
	error = sysctl_handle_int(oidp, &v, 0, req);
	if (error != 0 || req->newptr == NULL)
		return (error);
	if (v == 0 || athp_dev != NULL || athp_attaching)
		return (0);
	athp_attaching = 1;
	ksnprintf(wlan_status, sizeof(wlan_status), "starting");
	if (kthread_create(athp_dfly_attach_thread, NULL, NULL,
	    "athp attach") != 0) {
		athp_attaching = 0;
		return (ENOMEM);
	}
	return (0);
}
SYSCTL_PROC(_hw_mi5, OID_AUTO, wlan_attach, CTLTYPE_INT | CTLFLAG_RW, 0, 0,
    sysctl_wlan_attach, "I", "Attach the Wi-Fi driver (athp)");
