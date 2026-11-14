/*
 * WPA Supplicant - Layer2 packet handling straight on BPF
 * (l2_packet_freebsd.c without libpcap: the DragonFly/arm64 port has no
 * libpcap, and EAPOL needs nothing pcap adds)
 *
 * Copyright (c) 2003-2005, Jouni Malinen <j@w1.fi>
 * Copyright (c) 2005, Sam Leffler <sam@errno.com>
 *
 * This software may be distributed under the terms of the BSD license.
 * See README for more details.
 */

#include "includes.h"
#include <sys/param.h>
#include <sys/ioctl.h>
#include <sys/sysctl.h>
#include <net/bpf.h>
#include <net/if.h>
#include <net/if_dl.h>
#include <net/route.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>

#include "common.h"
#include "eloop.h"
#include "l2_packet.h"

struct l2_packet_data {
	int fd;
	u_int buflen;
	u8 *buf;
	char ifname[100];
	u8 own_addr[ETH_ALEN];
	void (*rx_callback)(void *ctx, const u8 *src_addr,
			    const u8 *buf, size_t len);
	void *rx_callback_ctx;
	int l2_hdr; /* whether to include layer 2 (Ethernet) header data
		     * buffers */
};


int l2_packet_get_own_addr(struct l2_packet_data *l2, u8 *addr)
{
	os_memcpy(addr, l2->own_addr, ETH_ALEN);
	return 0;
}


int l2_packet_send(struct l2_packet_data *l2, const u8 *dst_addr, u16 proto,
		   const u8 *buf, size_t len)
{
	struct l2_ethhdr *eth;
	ssize_t ret;

	if (l2->l2_hdr)
		return write(l2->fd, buf, len) < 0 ? -1 : 0;
	eth = os_malloc(sizeof(*eth) + len);
	if (eth == NULL)
		return -1;
	os_memcpy(eth->h_dest, dst_addr, ETH_ALEN);
	os_memcpy(eth->h_source, l2->own_addr, ETH_ALEN);
	eth->h_proto = htons(proto);
	os_memcpy(eth + 1, buf, len);
	ret = write(l2->fd, eth, sizeof(*eth) + len);
	os_free(eth);
	return ret < 0 ? -1 : 0;
}


static void l2_packet_receive(int sock, void *eloop_ctx, void *sock_ctx)
{
	struct l2_packet_data *l2 = eloop_ctx;
	ssize_t n;
	u8 *p, *end;

	n = read(l2->fd, l2->buf, l2->buflen);
	if (n <= 0)
		return;
	for (p = l2->buf, end = l2->buf + n; p < end;) {
		struct bpf_hdr *bh = (struct bpf_hdr *) p;
		struct l2_ethhdr *eth;
		const u8 *data;
		size_t len;

		if (p + sizeof(*bh) > end ||
		    p + bh->bh_hdrlen + bh->bh_caplen > end)
			break;
		eth = (struct l2_ethhdr *) (p + bh->bh_hdrlen);
		p += BPF_WORDALIGN(bh->bh_hdrlen + bh->bh_caplen);
		if (bh->bh_caplen < sizeof(*eth))
			continue;
		/* not our own frames */
		if (os_memcmp(eth->h_source, l2->own_addr, ETH_ALEN) == 0)
			continue;
		if (l2->l2_hdr) {
			data = (const u8 *) eth;
			len = bh->bh_caplen;
		} else {
			data = (const u8 *) (eth + 1);
			len = bh->bh_caplen - sizeof(*eth);
		}
		l2->rx_callback(l2->rx_callback_ctx, eth->h_source, data, len);
	}
}


static int open_bpf(void)
{
	char dev[32];
	int fd, i;

	fd = open("/dev/bpf", O_RDWR);
	if (fd >= 0)
		return fd;
	for (i = 0; i < 64; i++) {
		os_snprintf(dev, sizeof(dev), "/dev/bpf%d", i);
		fd = open(dev, O_RDWR);
		if (fd >= 0 || errno != EBUSY)
			return fd;
	}
	return -1;
}


static int l2_packet_init_bpf(struct l2_packet_data *l2,
			      unsigned short protocol)
{
	/* ether proto == protocol */
	struct bpf_insn insns[] = {
		BPF_STMT(BPF_LD + BPF_H + BPF_ABS, 12),
		BPF_JUMP(BPF_JMP + BPF_JEQ + BPF_K, protocol, 0, 1),
		BPF_STMT(BPF_RET + BPF_K, (u_int) -1),
		BPF_STMT(BPF_RET + BPF_K, 0),
	};
	struct bpf_program prog;
	struct ifreq ifr;
	u_int on = 1, blen = 4096;

	l2->fd = open_bpf();
	if (l2->fd < 0) {
		wpa_printf(MSG_ERROR, "l2_packet: no bpf device: %s",
			   strerror(errno));
		return -1;
	}
	(void) ioctl(l2->fd, BIOCSBLEN, &blen);
	os_memset(&ifr, 0, sizeof(ifr));
	os_strlcpy(ifr.ifr_name, l2->ifname, sizeof(ifr.ifr_name));
	if (ioctl(l2->fd, BIOCSETIF, &ifr) < 0) {
		wpa_printf(MSG_ERROR, "l2_packet: BIOCSETIF %s: %s",
			   l2->ifname, strerror(errno));
		goto fail;
	}
	if (ioctl(l2->fd, BIOCGBLEN, &blen) < 0)
		blen = 4096;
	(void) ioctl(l2->fd, BIOCIMMEDIATE, &on);
	(void) ioctl(l2->fd, BIOCSHDRCMPLT, &on);	/* our source address */
	prog.bf_len = ARRAY_SIZE(insns);
	prog.bf_insns = insns;
	if (ioctl(l2->fd, BIOCSETF, &prog) < 0) {
		wpa_printf(MSG_ERROR, "l2_packet: BIOCSETF: %s",
			   strerror(errno));
		goto fail;
	}
	l2->buflen = blen;
	l2->buf = os_malloc(blen);
	if (l2->buf == NULL)
		goto fail;
	eloop_register_read_sock(l2->fd, l2_packet_receive, l2, NULL);
	return 0;
fail:
	close(l2->fd);
	l2->fd = -1;
	return -1;
}


static int eth_get(const char *device, u8 ea[ETH_ALEN])
{
	struct if_msghdr *ifm;
	struct sockaddr_dl *sdl;
	u_char *p, *buf;
	size_t len;
	int mib[] = { CTL_NET, AF_ROUTE, 0, AF_LINK, NET_RT_IFLIST, 0 };

	if (sysctl(mib, 6, NULL, &len, NULL, 0) < 0)
		return -1;
	if ((buf = os_malloc(len)) == NULL)
		return -1;
	if (sysctl(mib, 6, buf, &len, NULL, 0) < 0) {
		os_free(buf);
		return -1;
	}
	for (p = buf; p < buf + len; p += ifm->ifm_msglen) {
		ifm = (struct if_msghdr *)p;
		sdl = (struct sockaddr_dl *)(ifm + 1);
		if (ifm->ifm_type != RTM_IFINFO ||
		    (ifm->ifm_addrs & RTA_IFP) == 0)
			continue;
		if (sdl->sdl_family != AF_LINK || sdl->sdl_nlen == 0 ||
		    sdl->sdl_nlen != os_strlen(device) ||
		    os_memcmp(sdl->sdl_data, device, sdl->sdl_nlen) != 0)
			continue;
		os_memcpy(ea, LLADDR(sdl), sdl->sdl_alen);
		break;
	}
	os_free(buf);

	if (p >= buf + len) {
		errno = ESRCH;
		return -1;
	}
	return 0;
}


struct l2_packet_data * l2_packet_init(
	const char *ifname, const u8 *own_addr, unsigned short protocol,
	void (*rx_callback)(void *ctx, const u8 *src_addr,
			    const u8 *buf, size_t len),
	void *rx_callback_ctx, int l2_hdr)
{
	struct l2_packet_data *l2;

	l2 = os_zalloc(sizeof(struct l2_packet_data));
	if (l2 == NULL)
		return NULL;
	os_strlcpy(l2->ifname, ifname, sizeof(l2->ifname));
	l2->rx_callback = rx_callback;
	l2->rx_callback_ctx = rx_callback_ctx;
	l2->l2_hdr = l2_hdr;

	if (eth_get(l2->ifname, l2->own_addr) < 0) {
		fprintf(stderr, "Failed to get link-level address for "
			"interface '%s'.\n", l2->ifname);
		os_free(l2);
		return NULL;
	}

	if (l2_packet_init_bpf(l2, protocol)) {
		os_free(l2);
		return NULL;
	}

	return l2;
}


struct l2_packet_data * l2_packet_init_bridge(
	const char *br_ifname, const char *ifname, const u8 *own_addr,
	unsigned short protocol,
	void (*rx_callback)(void *ctx, const u8 *src_addr,
			    const u8 *buf, size_t len),
	void *rx_callback_ctx, int l2_hdr)
{
	return l2_packet_init(br_ifname, own_addr, protocol, rx_callback,
			      rx_callback_ctx, l2_hdr);
}


void l2_packet_deinit(struct l2_packet_data *l2)
{
	if (l2 == NULL)
		return;
	if (l2->fd >= 0) {
		eloop_unregister_read_sock(l2->fd);
		close(l2->fd);
	}
	os_free(l2->buf);
	os_free(l2);
}


int l2_packet_get_ip_addr(struct l2_packet_data *l2, char *buf, size_t len)
{
	return -1;	/* used only for status display */
}


void l2_packet_notify_auth_start(struct l2_packet_data *l2)
{
}


int l2_packet_set_packet_filter(struct l2_packet_data *l2,
				enum l2_packet_filter_type type)
{
	return -1;
}
