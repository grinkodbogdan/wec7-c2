/*
 * /dev/fb0 test: draw colour bars and a gradient, flush, report.
 */
#include <sys/types.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <machine/fbmem.h>
#include <err.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <unistd.h>

static void
put(uint8_t *p, int bpp, uint8_t r, uint8_t g, uint8_t b)
{
	if (bpp == 32) {
		p[0] = b; p[1] = g; p[2] = r; p[3] = 0xff;
	} else if (bpp == 24) {
		p[0] = r; p[1] = g; p[2] = b;	/* the Mi 5's RGB888 */
	} else {
		*(uint16_t *)p = (r >> 3) << 11 | (g >> 2) << 5 | b >> 3;
	}
}

int
main(int argc, char **argv)
{
	static const uint8_t bars[8][3] = {
		{255,255,255}, {255,255,0}, {0,255,255}, {0,255,0},
		{255,0,255}, {255,0,0}, {0,0,255}, {0,0,0} };
	struct fbmem_info fi;
	uint8_t *fb;
	uint32_t x, y;
	int fd, secs = argc > 1 ? atoi(argv[1]) : 3;

	if ((fd = open("/dev/fb0", O_RDWR)) < 0)
		err(1, "/dev/fb0");
	if (ioctl(fd, FBMEM_GETINFO, &fi) < 0)
		err(1, "FBMEM_GETINFO");
	fb = mmap(NULL, fi.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
	if (fb == MAP_FAILED)
		err(1, "mmap");
	for (y = 0; y < fi.height; y++)
		for (x = 0; x < fi.width; x++) {
			uint8_t *p = fb + y * fi.stride + x * (fi.bpp / 8);
			if (y < fi.height / 2) {
				const uint8_t *c = bars[x * 8 / fi.width];
				put(p, fi.bpp, c[0], c[1], c[2]);
			} else {
				uint8_t v = x * 255 / fi.width;
				put(p, fi.bpp, v, (y - fi.height / 2) * 255 /
				    (fi.height / 2), 255 - v);
			}
		}
	if (ioctl(fd, FBMEM_FLUSH) < 0)
		err(1, "FBMEM_FLUSH");
	printf("fb0: %ux%u, %u bpp, drew colour bars\n", fi.width, fi.height,
	    fi.bpp);
	sleep(secs);
	close(fd);
	return (0);
}
