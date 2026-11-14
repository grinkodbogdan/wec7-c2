/*
 * Copyright (c) 2026 The DragonFly Project.  All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 * 1. Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in
 *    the documentation and/or other materials provided with the
 *    distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
 * ANY EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.  IN NO EVENT SHALL THE
 * AUTHOR OR CONTRIBUTORS BE LIABLE FOR ANY DAMAGES ARISING IN ANY WAY OUT
 * OF THE USE OF THIS SOFTWARE.
 */

/*
 * MSM8996's RPM (resource power manager): requests over GLINK.
 *
 * The RPM owns the shared clocks and buses: the crypto engine the secure
 * world uses to verify signed images, the system and memory buses, and
 * more.  Nothing on our side has asked for them since the bootloader
 * left, so this does what Linux's RPM clock driver does when it starts
 * (clk_smd_rpm_handoff in drivers/clk/qcom/clk-smd-rpm.c): ask for each
 * at its maximum, for both the active and the sleep set.
 *
 * The transport follows Linux's drivers/rpmsg/qcom_glink_rpm.c and
 * qcom_glink_native.c (intentless GLINK over the RPM message RAM: a
 * table of contents at its end names a transmit and a receive FIFO,
 * each a tail word, a head word and the data; the RPM is kicked through
 * the APCS IPC register), the messages drivers/soc/qcom/smd-rpm.c.  It is
 * polled, from a kernel thread, rather than interrupt driven.
 * Progress is kept in sysctl hw.mi5.rpm.
 */

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/sysctl.h>
#include <sys/lock.h>

#include <vm/vm.h>
#include <vm/pmap.h>
#include <machine/cpufunc.h>
#include <machine/md_var.h>
#include <machine/vmparam.h>

#define	MSG_RAM_PA		0x00068000UL
#define	MSG_RAM_SIZE		0x6000
#define	TOC_SIZE		256
#define	TOC_MAGIC		0x67727430	/* "grt0" */
#define	TX_FIFO_ID		0x61703272	/* "ap2r" */
#define	RX_FIFO_ID		0x72326170	/* "r2ap" */
#define	APCS_IPC		(0x09820000UL + 16)	/* bit 0: the RPM */

#define	CMD_VERSION		0
#define	CMD_VERSION_ACK		1
#define	CMD_OPEN		2
#define	CMD_CLOSE		3
#define	CMD_OPEN_ACK		4
#define	CMD_INTENT		5
#define	CMD_RX_DONE		6
#define	CMD_RX_INTENT_REQ	7
#define	CMD_RX_INTENT_REQ_ACK	8
#define	CMD_TX_DATA		9
#define	CMD_CLOSE_ACK		11
#define	CMD_TX_DATA_CONT	12
#define	CMD_READ_NOTIF		13
#define	CMD_RX_DONE_W_REUSE	14
#define	CMD_SIGNALS		15

#define	RPM_SERVICE_REQ		0x00716572	/* "req" */
#define	RPM_MSG_ERR		0x00727265	/* "err" */
#define	RPM_MSG_ID		0x2367736d	/* "msg#" */
#define	KEY_RATE		0x007a484b	/* "KHz" */
#define	KEY_ENABLE		0x62616e45	/* "Enab" */
#define	RPM_ACTIVE		0
#define	RPM_SLEEP		1

#define	CHANNEL			"rpm_requests"
#define	LCID			1

static volatile uint32_t *ram;		/* the message RAM, word access */
static struct pipe {
	uint32_t	off;		/* of the tail word, in bytes */
	uint32_t	len;
} tx, rx;
static struct lock rpm_lock;
static int rpm_ready, rpm_rcid = -1, rpm_open_acked, rpm_versioned;
static int rpm_reply = 1;		/* 0 ack, <0 error, 1 waiting */
static uint32_t rpm_msg_id = 1;
static volatile int rpm_handoff_done;

static char rpm_status[2048] = "not started";
static size_t rpm_status_len;
SYSCTL_DECL(_hw_mi5);
SYSCTL_STRING(_hw_mi5, OID_AUTO, rpm, CTLFLAG_RD, rpm_status, 0,
    "RPM link progress");

static void
st(const char *fmt, ...)
{
	char line[200];
	__va_list ap;
	size_t n;

	__va_start(ap, fmt);
	kvsnprintf(line, sizeof(line), fmt, ap);
	__va_end(ap);
	n = strlen(line);
	if (rpm_status_len + n + 2 < sizeof(rpm_status)) {
		rpm_status[rpm_status_len++] = '\n';
		memcpy(rpm_status + rpm_status_len, line, n + 1);
		rpm_status_len += n;
	}
	kprintf("rpm: %s\n", line);
}

static __inline uint32_t
w(uint32_t byteoff)
{
	return (ram[byteoff / 4]);
}

static __inline void
setw(uint32_t byteoff, uint32_t v)
{
	ram[byteoff / 4] = v;
}

#define	TAIL(p)		w((p)->off)
#define	HEAD(p)		w((p)->off + 4)
#define	FIFO(p)		((p)->off + 8)

static void
kick(void)
{
	cpu_sfence();
	*(volatile uint32_t *)PHYS_TO_DMAP(APCS_IPC) = 1;
}

/* ---- receive ---- */

static uint32_t
rx_avail(void)
{
	uint32_t head = HEAD(&rx), tail = TAIL(&rx);

	return (head >= tail ? head - tail : rx.len - tail + head);
}

/* (count) bytes at (offset) past the tail, a multiple of 4 */
static void
rx_peek(void *buf, uint32_t offset, uint32_t count)
{
	uint32_t *d = buf, tail = TAIL(&rx) + offset;

	if (tail >= rx.len)
		tail -= rx.len;
	for (; count >= 4; count -= 4, ++d) {
		*d = w(FIFO(&rx) + tail);
		tail += 4;
		if (tail >= rx.len)
			tail = 0;
	}
}

static void
rx_advance(uint32_t count)
{
	uint32_t tail = TAIL(&rx) + count;

	if (tail >= rx.len)
		tail -= rx.len;
	setw(rx.off, tail);
}

/* ---- transmit ---- */

static uint32_t
tx_avail(void)
{
	uint32_t head = HEAD(&tx), tail = TAIL(&tx);

	return (tail <= head ? tx.len - head + tail : tail - head);
}

/* Words out; the total padded to 8 bytes as glink_rpm_tx_write does. */
static int
tx_write(const uint32_t *hdr, size_t hlen, const uint32_t *data, size_t dlen)
{
	size_t total = roundup2(hlen + dlen, 8), i;
	uint32_t head;
	int tries;

	for (tries = 0; tx_avail() <= total; ++tries) {
		if (tries > 1000)
			return (EBUSY);
		DELAY(100);
	}
	head = HEAD(&tx);
	for (i = 0; i < total; i += 4) {
		uint32_t v = 0;

		if (i < hlen)
			v = hdr[i / 4];
		else if (i - hlen < dlen)
			v = data[(i - hlen) / 4];
		setw(FIFO(&tx) + head, v);
		head += 4;
		if (head >= tx.len)
			head = 0;
	}
	cpu_sfence();
	setw(tx.off + 4, head);
	kick();
	return (0);
}

static int
send_cmd(uint16_t cmd, uint16_t p1, uint32_t p2)
{
	uint32_t m[2] = { cmd | ((uint32_t)p1 << 16), p2 };

	return (tx_write(m, sizeof(m), NULL, 0));
}

/* ---- the RPM's replies ---- */

static void
rpm_message(const uint8_t *buf, uint32_t len)
{
	uint32_t svc, hlen, off, type, mlen;

	if (len < 8)
		return;
	memcpy(&svc, buf, 4);
	memcpy(&hlen, buf + 4, 4);
	if (svc != RPM_SERVICE_REQ)
		return;
	for (off = 8; off + 8 <= len && off < 8 + hlen;) {
		memcpy(&type, buf + off, 4);
		memcpy(&mlen, buf + off + 4, 4);
		if (type == RPM_MSG_ERR) {
			char err[40];
			size_t n = MIN(mlen, sizeof(err) - 1);

			memcpy(err, buf + off + 8, MIN(n, len - off - 8));
			err[n] = 0;
			st("rpm error: %s", err);
			rpm_reply = -1;
			return;
		}
		off += roundup2(8 + mlen, 4);
	}
	rpm_reply = 0;
}

/* Handle everything in the receive FIFO. */
static void
rx_poll(void)
{
	uint32_t m[2], avail, cmd, p1, p2;

	for (;;) {
		avail = rx_avail();
		if (avail < 8)
			return;
		rx_peek(m, 0, 8);
		cmd = m[0] & 0xffff;
		p1 = m[0] >> 16;
		p2 = m[1];
		switch (cmd) {
		case CMD_VERSION:
			rx_advance(8);
			send_cmd(CMD_VERSION_ACK, 1, 0);
			rpm_versioned = 1;
			break;
		case CMD_VERSION_ACK:
			rx_advance(8);
			rpm_versioned = 1;
			break;
		case CMD_OPEN: {
			char name[40];
			uint32_t n = roundup2(8 + p2, 8);

			if (avail < n)
				return;
			memset(name, 0, sizeof(name));
			rx_peek(name, 8, MIN(roundup2(p2, 4), sizeof(name) - 4));
			rx_advance(n);
			if (strcmp(name, CHANNEL) == 0) {
				rpm_rcid = p1;
				send_cmd(CMD_OPEN_ACK, p1, 0);
			}
			break;
		}
		case CMD_OPEN_ACK:
			rx_advance(8);
			if (p1 == LCID)
				rpm_open_acked = 1;
			break;
		case CMD_CLOSE:
			rx_advance(8);
			send_cmd(CMD_CLOSE_ACK, p1, 0);
			if ((int)p1 == rpm_rcid)
				rpm_rcid = -1;
			break;
		case CMD_INTENT:
			if (avail < roundup2(8 + 8 * p2, 8))
				return;
			rx_advance(roundup2(8 + 8 * p2, 8));
			break;
		case CMD_TX_DATA:
		case CMD_TX_DATA_CONT: {
			uint32_t h[4], chunk;
			uint8_t buf[256];

			if (avail < 16)
				return;
			rx_peek(h, 0, 16);
			chunk = h[2];
			if (avail < 16 + chunk)
				return;
			if (chunk <= sizeof(buf)) {
				rx_peek(buf, 16, roundup2(chunk, 4));
				rpm_message(buf, chunk);
			}
			rx_advance(roundup2(16 + chunk, 8));
			break;
		}
		case CMD_READ_NOTIF:
			rx_advance(8);
			kick();
			break;
		case CMD_RX_DONE:
		case CMD_RX_DONE_W_REUSE:
		case CMD_RX_INTENT_REQ_ACK:
		case CMD_CLOSE_ACK:
		case CMD_SIGNALS:
		case CMD_RX_INTENT_REQ:
			rx_advance(8);
			break;
		default:
			st("unknown command %u from the RPM; resetting rx", cmd);
			setw(rx.off, HEAD(&rx));
			return;
		}
	}
}

/* Poll until (*flag) becomes (want) or about (ms) pass. */
static int
wait_for(volatile int *flag, int want, int ms)
{
	int i;

	for (i = 0; i < ms * 10; ++i) {
		rx_poll();
		if (*flag == want)
			return (0);
		DELAY(100);
	}
	return (ETIMEDOUT);
}

/*
 * One request (qcom_rpm_smd_write): resource type and id, one key and
 * a 32-bit value, for the active or sleep set.  0, or an error.
 */
int
msm8996_rpm_write(int state, uint32_t type, uint32_t id, uint32_t key,
    uint32_t value)
{
	uint32_t pkt[4 + 2 + 5 + 3], dhdr[4];
	int error;

	if (!rpm_ready)
		return (ENXIO);
	lockmgr(&rpm_lock, LK_EXCLUSIVE);
	pkt[0] = RPM_SERVICE_REQ;
	pkt[1] = 5 * 4 + 12;		/* request + payload */
	pkt[2] = rpm_msg_id++;
	pkt[3] = state;
	pkt[4] = type;
	pkt[5] = id;
	pkt[6] = 12;
	pkt[7] = key;
	pkt[8] = 4;
	pkt[9] = value;
	dhdr[0] = CMD_TX_DATA | ((uint32_t)LCID << 16);
	dhdr[1] = 0;			/* intent: none, intentless */
	dhdr[2] = 10 * 4;		/* this chunk */
	dhdr[3] = 0;			/* nothing left */
	rpm_reply = 1;
	error = tx_write(dhdr, sizeof(dhdr), pkt, 10 * 4);
	if (error == 0)
		error = wait_for((volatile int *)&rpm_reply, 0, 1000);
	if (error == 0 && rpm_reply < 0)
		error = EIO;
	if (error == ETIMEDOUT && rpm_reply < 0)
		error = EIO;
	lockmgr(&rpm_lock, LK_RELEASE);
	return (error);
}

int
msm8996_rpm_ready(void)
{
	return (rpm_handoff_done);
}

/* The resources Linux's rpmcc hands off at its maximum on MSM8996. */
static const struct {
	const char	*name;
	uint32_t	type, id, key, value;
} handoff[] = {
	{ "ce1",	0x6563,     0, KEY_RATE, 0x7fffffff },	/* crypto engine */
	{ "pcnoc",	0x316b6c63, 0, KEY_RATE, 0x7fffffff },	/* bus clk1 */
	{ "snoc",	0x316b6c63, 1, KEY_RATE, 0x7fffffff },
	{ "cnoc",	0x316b6c63, 2, KEY_RATE, 0x7fffffff },
	{ "bimc",	0x326b6c63, 0, KEY_RATE, 0x7fffffff },	/* mem clk2 */
	{ "aggre1",	0x72676761, 1, KEY_RATE, 0x7fffffff },
	{ "aggre2",	0x72676761, 2, KEY_RATE, 0x7fffffff },
	{ "mmaxi",	0x69786d6d, 0, KEY_RATE, 0x7fffffff },	/* mmssnoc axi */
};

static void
rpm_thread(void *arg __unused)
{
	uint32_t toc[TOC_SIZE / 4];
	int i, n, error;

	ram = pmap_mapdev(MSG_RAM_PA, MSG_RAM_SIZE);
	for (i = 0; i < TOC_SIZE / 4; ++i)
		toc[i] = w(MSG_RAM_SIZE - TOC_SIZE + 4 * i);
	if (toc[0] != TOC_MAGIC) {
		st("no GLINK table in the RPM message RAM (0x%08x)", toc[0]);
		goto out;
	}
	n = toc[1];
	for (i = 0; i < n && 2 + 3 * i + 2 < TOC_SIZE / 4; ++i) {
		uint32_t id = toc[2 + 3 * i], off = toc[3 + 3 * i],
		    size = toc[4 + 3 * i];

		if (off + size > MSG_RAM_SIZE)
			continue;
		if (id == TX_FIFO_ID) {
			tx.off = off;
			tx.len = size;
		} else if (id == RX_FIFO_ID) {
			rx.off = off;
			rx.len = size;
		}
	}
	if (tx.len == 0 || rx.len == 0) {
		st("RPM FIFOs not found in the table");
		goto out;
	}
	st("fifos: tx 0x%x+0x%x (head %u tail %u), rx 0x%x+0x%x (head %u "
	    "tail %u)", tx.off, tx.len, HEAD(&tx), TAIL(&tx), rx.off, rx.len,
	    HEAD(&rx), TAIL(&rx));

	/* as glink_rpm_probe: start our ends afresh */
	setw(tx.off + 4, 0);
	setw(rx.off, 0);

	send_cmd(CMD_VERSION, 1, 0);
	if ((error = wait_for((volatile int *)&rpm_versioned, 1, 1000)) != 0) {
		st("no version reply from the RPM");
		goto out;
	}
	{
		uint32_t m[2 + 4];	/* OPEN, the name padded */

		memset(m, 0, sizeof(m));
		m[0] = CMD_OPEN | ((uint32_t)LCID << 16);
		m[1] = sizeof(CHANNEL);
		memcpy(&m[2], CHANNEL, sizeof(CHANNEL));
		tx_write(m, roundup2(8 + sizeof(CHANNEL), 8), NULL, 0);
	}
	if (wait_for((volatile int *)&rpm_open_acked, 1, 1000) != 0) {
		st("the RPM did not acknowledge opening %s", CHANNEL);
		goto out;
	}
	for (i = 0; i < 1000 && rpm_rcid < 0; ++i) {
		rx_poll();
		DELAY(1000);
	}
	st("channel %s open (local %d, remote %d)", CHANNEL, LCID, rpm_rcid);
	rpm_ready = 1;

	for (i = 0; i < (int)NELEM(handoff); ++i) {
		int ea = msm8996_rpm_write(RPM_ACTIVE, handoff[i].type,
		    handoff[i].id, handoff[i].key, handoff[i].value);
		int es = msm8996_rpm_write(RPM_SLEEP, handoff[i].type,
		    handoff[i].id, handoff[i].key, handoff[i].value);

		st("%s at maximum: active %d, sleep %d", handoff[i].name, ea,
		    es);
	}
	rpm_handoff_done = 1;
	st("ready");
out:
	kthread_exit();
}

static void
msm8996_rpm_init(void *dummy __unused)
{
	int enable = 1;

	if (!msm8996_is_present())
		return;
	TUNABLE_INT_FETCH("msm8996.rpm", &enable);
	if (!enable)
		return;
	lockinit(&rpm_lock, "rpm", 0, 0);
	kthread_create(rpm_thread, NULL, NULL, "rpm");
}
SYSINIT(msm8996rpm, SI_SUB_CONFIGURE, SI_ORDER_MIDDLE, msm8996_rpm_init, NULL);
