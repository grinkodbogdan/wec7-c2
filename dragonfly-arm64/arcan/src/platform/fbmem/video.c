/*
 * Copyright 2026, The DragonFly Project.
 * License: 3-Clause BSD, see COPYING file in arcan source repository.
 *
 * Description: video platform for a plain framebuffer and software GL,
 * written for DragonFly/arm64 on phones whose GPU has no driver yet.
 *
 * Rendering goes through Mesa's OSMesa (softpipe) into a client-memory
 * buffer at a fraction of the panel's resolution (ARCAN_FBMEM_SCALE,
 * default 2); each synch converts and scales the result into the
 * framebuffer from /dev/fb0 (ARCAN_FBMEM_DEVICE) and asks the kernel to
 * push it to the panel.  One display, one mode, no buffer sharing with
 * clients (they hand over shared memory, which agp uploads).
 *
 * Based on the egl-gles platform's structure.
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <math.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/ioctl.h>
#include <sys/mman.h>

#include <machine/fbmem.h>

#define GL_GLEXT_PROTOTYPES
#include <GL/osmesa.h>
#include <GL/gl.h>
#include <GL/glext.h>

#include "arcan_math.h"
#include "arcan_general.h"
#include "arcan_video.h"
#include "arcan_videoint.h"
#include "arcan_shmif.h"
#include "../platform.h"

static struct {
	int fd;
	struct fbmem_info fi;
	uint8_t *fb;
	OSMesaContext ctx;
	uint8_t *buf;			/* canvas, RGBA, top row first */
	size_t canvasw, canvash;
	unsigned scale;
	uint64_t frames;
} fbm = { .fd = -1 };

static char* fbm_envopts[] = {
	"ARCAN_FBMEM_DEVICE=/dev/fb0", "framebuffer device",
	"ARCAN_FBMEM_SCALE=2", "panel pixels per canvas pixel (1..4)",
	NULL
};

struct monitor_mode platform_video_dimensions()
{
	struct monitor_mode res = {
		.width = fbm.canvasw,
		.height = fbm.canvash
	};
	res.phy_width = (float) res.width / ARCAN_SHMPAGE_DEFAULT_PPCM * 10.0;
	res.phy_height = (float) res.height / ARCAN_SHMPAGE_DEFAULT_PPCM * 10.0;
	return res;
}

bool platform_video_set_mode(platform_display_id disp,
	platform_mode_id mode, struct platform_mode_opts opts)
{
	return disp == 0 && mode == 0;
}

size_t platform_video_decay()
{
	return 0;
}

ssize_t platform_video_map_display_layer(arcan_vobj_id id,
	platform_display_id disp, size_t layer_index, struct display_layer_cfg cfg)
{
	return -1;
}

bool platform_video_map_display(
	arcan_vobj_id vid, platform_display_id id, enum blitting_hint hint)
{
	return false;
}

void platform_video_invalidate_map(
	struct agp_vstore* vstore, struct agp_region region)
{
}

int platform_video_cardhandle(int cardn,
		int* buffer_method, size_t* metadata_sz, uint8_t** metadata)
{
	return -1;
}

bool platform_video_display_edid(platform_display_id did,
	char** out, size_t* sz)
{
	*out = NULL;
	*sz = 0;
	return false;
}

void platform_video_reset(int id, int swap)
{
}

bool platform_video_specify_mode(platform_display_id id,
	struct monitor_mode mode)
{
	return false;
}

void platform_video_preinit()
{
}

bool platform_video_map_buffer(
	struct agp_vstore* vs, struct agp_buffer_plane* planes, size_t n)
{
	return false;
}

size_t platform_video_export_vstore(
	struct agp_vstore* vs, struct agp_buffer_plane* planes, size_t n)
{
	return 0;
}

bool platform_video_init(uint16_t w, uint16_t h,
	uint8_t bpp, bool fs, bool frames, const char* caption)
{
	const char* dev = getenv("ARCAN_FBMEM_DEVICE");
	const char* sc = getenv("ARCAN_FBMEM_SCALE");

	fbm.fd = open(dev ? dev : "/dev/fb0", O_RDWR | O_CLOEXEC);
	if (-1 == fbm.fd){
		arcan_warning("(fbmem) couldn't open %s\n", dev ? dev : "/dev/fb0");
		return false;
	}
	if (-1 == ioctl(fbm.fd, FBMEM_GETINFO, &fbm.fi)){
		arcan_warning("(fbmem) FBMEM_GETINFO failed\n");
		return false;
	}
	if (fbm.fi.bpp != 32 && fbm.fi.bpp != 24 && fbm.fi.bpp != 16){
		arcan_warning("(fbmem) unsupported depth %u\n", fbm.fi.bpp);
		return false;
	}
	fbm.fb = mmap(NULL, fbm.fi.size,
		PROT_READ | PROT_WRITE, MAP_SHARED, fbm.fd, 0);
	if (MAP_FAILED == fbm.fb){
		arcan_warning("(fbmem) mmap failed\n");
		return false;
	}

	fbm.scale = sc ? strtoul(sc, NULL, 10) : 2;
	if (fbm.scale < 1 || fbm.scale > 4)
		fbm.scale = 2;
	fbm.canvasw = fbm.fi.width / fbm.scale;
	fbm.canvash = fbm.fi.height / fbm.scale;

/* depth 24, stencil 8: what the engine's render targets may ask for */
	fbm.ctx = OSMesaCreateContextExt(OSMESA_RGBA, 24, 8, 0, NULL);
	fbm.buf = malloc(fbm.canvasw * fbm.canvash * 4);
	if (!fbm.ctx || !fbm.buf ||
		!OSMesaMakeCurrent(fbm.ctx, fbm.buf, GL_UNSIGNED_BYTE,
			fbm.canvasw, fbm.canvash)){
		arcan_warning("(fbmem) couldn't create the OSMesa context\n");
		return false;
	}
/* top row first, as the framebuffer wants it */
	OSMesaPixelStore(OSMESA_Y_UP, 0);

	arcan_warning("(fbmem) %ux%u %ubpp panel, %zux%zu canvas, %s\n",
		fbm.fi.width, fbm.fi.height, fbm.fi.bpp,
		fbm.canvasw, fbm.canvash, (const char*) glGetString(GL_RENDERER));

	glViewport(0, 0, fbm.canvasw, fbm.canvash);
	agp_init();
	return true;
}

bool platform_video_auth(int cardn, unsigned token)
{
	return false;
}

struct monitor_mode* platform_video_query_modes(
	platform_display_id id, size_t* count)
{
	static struct monitor_mode mode = {};
	mode.width  = fbm.canvasw;
	mode.height = fbm.canvash;
	mode.depth  = sizeof(av_pixel) * 8;
	mode.refresh = 60;
	*count = 1;
	return &mode;
}

void platform_video_query_displays()
{
}

const char** platform_video_envopts()
{
	return (const char**) fbm_envopts;
}

#include "glsyms.h"

/*
 * OSMesaGetProcAddress only knows extension entry points; the core ones
 * the engine looks up by name are linked in statically.
 */
void* platform_video_gfxsym(const char* sym)
{
	for (size_t i = 0; i < sizeof(fbm_glsyms) / sizeof(fbm_glsyms[0]); i++)
		if (strcmp(fbm_glsyms[i].name, sym) == 0)
			return fbm_glsyms[i].fun;
	return (void*) OSMesaGetProcAddress(sym);
}

bool platform_video_map_handle(
	struct agp_vstore* dst, int64_t handle)
{
	return false;
}

enum dpms_state
	platform_video_dpms(platform_display_id did, enum dpms_state state)
{
	return ADPMS_ON;
}

const char* platform_video_capstr()
{
	static char* capstr;

	if (!capstr){
		const char* vendor = (const char*) glGetString(GL_VENDOR);
		const char* render = (const char*) glGetString(GL_RENDERER);
		const char* version = (const char*) glGetString(GL_VERSION);
		size_t sz = 1024;

		capstr = malloc(sz);
		if (capstr)
			snprintf(capstr, sz, "Video Platform (fbmem, OSMesa)\n"
				"Vendor: %s\nRenderer: %s\nGL Version: %s\n"
				"Panel: %ux%u %ubpp, canvas %zux%zu\n",
				vendor, render, version, fbm.fi.width, fbm.fi.height,
				fbm.fi.bpp, fbm.canvasw, fbm.canvash);
	}
	return capstr;
}

size_t platform_video_displays(platform_display_id* dids, size_t* lim)
{
	if (dids && lim && *lim > 0){
		dids[0] = 0;
	}
	return 1;
}

/*
 * Canvas (RGBA bytes) to framebuffer, each canvas pixel becoming a
 * scale x scale block.
 */
static void blit_to_fb()
{
	const unsigned s = fbm.scale;
	const size_t bypp = fbm.fi.bpp / 8;

	for (size_t y = 0; y < fbm.canvash; y++){
		const uint8_t* src = fbm.buf + y * fbm.canvasw * 4;
		uint8_t* row = fbm.fb + (y * s) * fbm.fi.stride;
		uint8_t* d = row;

		for (size_t x = 0; x < fbm.canvasw; x++, src += 4){
			for (unsigned k = 0; k < s; k++, d += bypp){
				switch (fbm.fi.bpp){
				case 32: /* XRGB8888 little-endian: B G R X */
					d[0] = src[2]; d[1] = src[1]; d[2] = src[0]; d[3] = 0xff;
				break;
				case 24: /* packed 24-bit, blue first (the Mi 5 as LK sets it up) */
					d[0] = src[2]; d[1] = src[1]; d[2] = src[0];
				break;
				default: /* RGB565 */
					*(uint16_t*)d = (src[0] >> 3) << 11 |
						(src[1] >> 2) << 5 | src[2] >> 3;
				}
			}
		}
		for (unsigned k = 1; k < s; k++)
			memcpy(row + k * fbm.fi.stride, row, fbm.canvasw * s * bypp);
	}
}

void platform_video_synch(uint64_t tick_count, float fract,
	video_synchevent pre, video_synchevent post)
{
	if (pre)
		pre();

	size_t nd;
	arcan_bench_register_cost( arcan_vint_refresh(fract, &nd) );

/* nothing changed: don't spend a software frame on it */
	if (nd == 0 && fbm.frames > 0){
		arcan_timesleep(16);
		goto out;
	}

	if (arcan_vint_worldrt()){
		agp_activate_rendertarget(NULL);
		arcan_vint_drawrt(arcan_vint_world(), 0, 0, fbm.canvasw, fbm.canvash);
		arcan_vint_drawcursor(true);
		arcan_vint_drawcursor(false);
	}
	glFinish();
	blit_to_fb();
	ioctl(fbm.fd, FBMEM_FLUSH);
	fbm.frames++;

out:
	if (post)
		post();
}

bool platform_video_set_display_gamma(platform_display_id did,
	size_t n_ramps, uint16_t* r, uint16_t* g, uint16_t* b)
{
	return false;
}

bool platform_video_get_display_gamma(platform_display_id did,
	size_t* n_ramps, uint16_t** outb)
{
	return false;
}

void platform_video_prepare_external() {}
void platform_video_restore_external() {}

void platform_video_shutdown()
{
	if (fbm.ctx)
		OSMesaDestroyContext(fbm.ctx);
	if (fbm.fb && fbm.fb != MAP_FAILED)
		munmap(fbm.fb, fbm.fi.size);
	if (fbm.fd != -1)
		close(fbm.fd);
	fbm.fd = -1;
}

void platform_video_minimize() {}
void platform_video_recovery() {}
