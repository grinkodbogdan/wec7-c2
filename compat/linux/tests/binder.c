/* binder.c - Android's Binder IPC on Flare, through the real UAPI
 * (<linux/android/binder.h>) and raw ioctls, as libbinder uses them.
 *
 * The parent becomes the context manager (handle 0, servicemanager's role).
 * A child sends it a transaction carrying a string, a binder object of its
 * own and a file descriptor; the parent replies, then calls back into the
 * child's object through the handle it was given; finally it asks for a
 * death notification and sees the child die. Prints "binder: ok". */
#include <fcntl.h>
#include <stdint.h>
#include <linux/android/binder.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#define MAP_SIZE (128 * 1024)
#define SERVICE_PTR 0x5e5e1ceull
#define SERVICE_COOKIE 0xc00c1eull

static int fails;
#define CHECK(c, what) do { if (!(c)) { printf("binder: FAILED %s\n", what); fails++; } else printf("binder: %s ok\n", what); } while (0)

static int bopen(void) {
    int fd = open("/dev/binder", O_RDWR | O_CLOEXEC);
    if (fd < 0 || mmap(NULL, MAP_SIZE, PROT_READ, MAP_PRIVATE, fd, 0) == MAP_FAILED) { perror("binder"); exit(2); }
    return fd;
}

static int write_read(int fd, void *w, size_t wlen, void *r, size_t rlen, size_t *got) {
    struct binder_write_read bwr = { .write_size = wlen, .write_buffer = (binder_uintptr_t)w,
                                     .read_size = rlen, .read_buffer = (binder_uintptr_t)r };
    int ret = ioctl(fd, BINDER_WRITE_READ, &bwr);
    if (got) *got = bwr.read_consumed;
    return ret;
}

/* Read until a transaction, reply or failure arrives; returns its BR code. */
static uint32_t wait_for(int fd, struct binder_transaction_data *out, binder_uintptr_t *cookie) {
    static uint8_t buf[512];
    for (;;) {
        size_t got = 0;
        if (write_read(fd, NULL, 0, buf, sizeof buf, &got) < 0) return 0;
        for (size_t off = 0; off + 4 <= got;) {
            uint32_t cmd = *(uint32_t *)(buf + off);
            off += 4;
            switch (cmd) {
            case BR_NOOP: case BR_TRANSACTION_COMPLETE: break;
            case BR_TRANSACTION: case BR_REPLY:
                memcpy(out, buf + off, sizeof *out); return cmd;
            case BR_DEAD_BINDER:
                *cookie = *(binder_uintptr_t *)(buf + off); return cmd;
            case BR_DEAD_REPLY: case BR_FAILED_REPLY: return cmd;
            default: printf("binder: unexpected %#x\n", cmd); return cmd;
            }
        }
    }
}

static void free_buffer(int fd, binder_uintptr_t buffer) {
    struct { uint32_t cmd; binder_uintptr_t buf; } __attribute__((packed)) c = { BC_FREE_BUFFER, buffer };
    write_read(fd, &c, sizeof c, NULL, 0, NULL);
}

/* Send a transaction or reply with `text` and optional objects. */
static void send_txn(int fd, uint32_t cmd, uint32_t handle, uint32_t code, const char *text,
                     struct flat_binder_object *objs, int nobjs) {
    static uint8_t data[256];
    static binder_size_t offs[4];
    size_t len = (strlen(text) + 8) & ~7ul;
    memset(data, 0, sizeof data);
    strcpy((char *)data, text);
    for (int i = 0; i < nobjs; i++) {
        offs[i] = len;
        memcpy(data + len, &objs[i], sizeof objs[i]);
        len += sizeof objs[i];
    }
    struct { uint32_t cmd; struct binder_transaction_data tr; } __attribute__((packed)) w;
    memset(&w, 0, sizeof w);
    w.cmd = cmd;
    w.tr.target.handle = handle;
    w.tr.code = code;
    w.tr.data_size = len;
    w.tr.offsets_size = nobjs * sizeof(binder_size_t);
    w.tr.data.ptr.buffer = (binder_uintptr_t)data;
    w.tr.data.ptr.offsets = (binder_uintptr_t)offs;
    write_read(fd, &w, sizeof w, NULL, 0, NULL);
}

static struct flat_binder_object *obj_at(struct binder_transaction_data *tr, int i) {
    binder_size_t *offs = (binder_size_t *)(uintptr_t)tr->data.ptr.offsets;
    return (struct flat_binder_object *)((uintptr_t)tr->data.ptr.buffer + offs[i]);
}

int main(void) {
    struct binder_version v = { 0 };
    int mgr = bopen();
    CHECK(ioctl(mgr, BINDER_VERSION, &v) == 0 && v.protocol_version == BINDER_CURRENT_PROTOCOL_VERSION, "version");
    CHECK(ioctl(mgr, BINDER_SET_CONTEXT_MGR, 0) == 0, "context manager");
    CHECK(ioctl(bopen(), BINDER_SET_CONTEXT_MGR, 0) == -1, "only one context manager");

    int pipefd[2];
    if (pipe(pipefd)) return 1;
    pid_t child = fork();
    if (child == 0) {
        /* The client: register with the manager, then serve our object. */
        close(mgr);
        int fd = bopen();
        struct flat_binder_object o[2] = {
            { .hdr.type = BINDER_TYPE_BINDER, .binder = SERVICE_PTR, .cookie = SERVICE_COOKIE },
            { .hdr.type = BINDER_TYPE_FD, .handle = pipefd[1] },
        };
        send_txn(fd, BC_TRANSACTION, 0, 1, "register", o, 2);
        struct binder_transaction_data tr;
        binder_uintptr_t ck;
        if (wait_for(fd, &tr, &ck) != BR_REPLY) _exit(3);
        int ok = !strcmp((char *)(uintptr_t)tr.data.ptr.buffer, "welcome");
        free_buffer(fd, tr.data.ptr.buffer);
        uint32_t enter = BC_ENTER_LOOPER;
        write_read(fd, &enter, 4, NULL, 0, NULL);
        /* A call to our own object, made through the manager's handle. */
        if (wait_for(fd, &tr, &ck) != BR_TRANSACTION) _exit(4);
        ok = ok && tr.target.ptr == SERVICE_PTR && tr.cookie == SERVICE_COOKIE && tr.code == 7
             && !strcmp((char *)(uintptr_t)tr.data.ptr.buffer, "ping");
        free_buffer(fd, tr.data.ptr.buffer);
        send_txn(fd, BC_REPLY, 0, 0, ok ? "pong" : "bad", NULL, 0);
        usleep(50000);
        _exit(ok ? 0 : 5);
    }
    close(pipefd[1]);

    /* The manager receives the registration. */
    struct binder_transaction_data tr;
    binder_uintptr_t ck = 0;
    CHECK(wait_for(mgr, &tr, &ck) == BR_TRANSACTION && tr.code == 1 && tr.sender_pid == child
          && !strcmp((char *)(uintptr_t)tr.data.ptr.buffer, "register"), "transaction to handle 0");
    struct flat_binder_object *svc = obj_at(&tr, 0), *fdo = obj_at(&tr, 1);
    uint32_t handle = svc->handle;
    CHECK(svc->hdr.type == BINDER_TYPE_HANDLE && handle != 0, "object became a handle");
    int got_fd = (int)fdo->handle;
    CHECK(fdo->hdr.type == BINDER_TYPE_FD && got_fd >= 0 && write(got_fd, "via binder", 10) == 10, "fd passed");
    char buf[32] = { 0 };
    CHECK(read(pipefd[0], buf, sizeof buf - 1) == 10 && !strcmp(buf, "via binder"), "fd reaches the same pipe");
    free_buffer(mgr, tr.data.ptr.buffer);
    send_txn(mgr, BC_REPLY, 0, 0, "welcome", NULL, 0);

    /* Call the child's object back through its handle. */
    send_txn(mgr, BC_TRANSACTION, handle, 7, "ping", NULL, 0);
    CHECK(wait_for(mgr, &tr, &ck) == BR_REPLY && !strcmp((char *)(uintptr_t)tr.data.ptr.buffer, "pong"),
          "call into a remote object and reply");
    free_buffer(mgr, tr.data.ptr.buffer);

    /* Death notification. */
    struct { uint32_t cmd; struct binder_handle_cookie hc; } __attribute__((packed)) dn =
        { BC_REQUEST_DEATH_NOTIFICATION, { handle, 0xdeadull } };
    write_read(mgr, &dn, sizeof dn, NULL, 0, NULL);
    int st = 0;
    waitpid(child, &st, 0);
    CHECK(WIFEXITED(st) && WEXITSTATUS(st) == 0, "client served its object");
    CHECK(wait_for(mgr, &tr, &ck) == BR_DEAD_BINDER && ck == 0xdeadull, "death notification");

    printf("binder: %s\n", fails ? "FAILED" : "ok");
    return fails ? 1 : 0;
}
