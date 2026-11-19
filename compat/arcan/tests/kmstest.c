/* kmstest: render with Mesa (GBM + EGL + GLES2) and scan out through KMS on
 * /dev/dri/card0, timing each step. Exercises the same path as Arcan's
 * egl-dri platform. */
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <fcntl.h>
#include <gbm.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

static double t0;
static double now(void) {
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec * 1000.0 + ts.tv_nsec / 1e6;
}
#define STEP(...) do { printf("kmstest: %8.1f ms: ", now() - t0); printf(__VA_ARGS__); printf("\n"); fflush(stdout); } while (0)
#define FAIL(...) do { STEP(__VA_ARGS__); return 1; } while (0)

static void flip_done(int fd, unsigned seq, unsigned s, unsigned us, void* data) { *(int*)data = 1; }

int main(void) {
	t0 = now();
	int fd = open("/dev/dri/card0", O_RDWR | O_CLOEXEC);
	if (fd < 0) FAIL("cannot open /dev/dri/card0");
	drmVersionPtr v = drmGetVersion(fd);
	STEP("driver %s: %s", v ? v->name : "?", v ? v->desc : "?");
	drmModeRes* res = drmModeGetResources(fd);
	if (!res || !res->count_connectors) FAIL("no KMS resources");
	drmModeConnector* conn = drmModeGetConnector(fd, res->connectors[0]);
	if (!conn || conn->connection != DRM_MODE_CONNECTED || !conn->count_modes) FAIL("no connected display");
	drmModeModeInfo mode = conn->modes[0];
	STEP("connector %u: %s, %ux%u mm", conn->connector_id, mode.name, conn->mmWidth, conn->mmHeight);
	struct gbm_device* gbm = gbm_create_device(fd);
	if (!gbm) FAIL("gbm_create_device failed");
	STEP("gbm backend %s", gbm_device_get_backend_name(gbm));
	EGLDisplay dpy = eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, gbm, NULL);
	EGLint maj, min;
	if (!eglInitialize(dpy, &maj, &min)) FAIL("eglInitialize failed: %#x", eglGetError());
	STEP("EGL %d.%d %s", maj, min, eglQueryString(dpy, EGL_VENDOR));
	eglBindAPI(EGL_OPENGL_ES_API);
	EGLint cattr[] = {EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE};
	EGLConfig cfgs[64];
	EGLint n = 0;
	eglChooseConfig(dpy, cattr, cfgs, 64, &n);
	EGLConfig cfg = NULL;
	for (int i = 0; i < n; i++) {
		EGLint id;
		eglGetConfigAttrib(dpy, cfgs[i], EGL_NATIVE_VISUAL_ID, &id);
		if (id == GBM_FORMAT_XRGB8888) { cfg = cfgs[i]; break; }
	}
	if (!cfg) FAIL("no XRGB8888 config among %d", n);
	EGLint ctxattr[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctxattr);
	if (!ctx) FAIL("eglCreateContext failed: %#x", eglGetError());
	struct gbm_surface* gs = gbm_surface_create(gbm, mode.hdisplay, mode.vdisplay, GBM_FORMAT_XRGB8888,
		GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING);
	EGLSurface surf = eglCreateWindowSurface(dpy, cfg, (EGLNativeWindowType)gs, NULL);
	if (!eglMakeCurrent(dpy, surf, surf, ctx)) FAIL("eglMakeCurrent failed: %#x", eglGetError());
	STEP("GL renderer %s, version %s", glGetString(GL_RENDERER), glGetString(GL_VERSION));
	uint32_t prev_fb = 0;
	struct gbm_bo* prev_bo = NULL;
	for (int frame = 0; frame < 3; frame++) {
		glClearColor(0.1f + 0.3f * frame, 0.4f, 0.8f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		eglSwapBuffers(dpy, surf);
		struct gbm_bo* bo = gbm_surface_lock_front_buffer(gs);
		uint32_t fb;
		if (drmModeAddFB(fd, mode.hdisplay, mode.vdisplay, 24, 32, gbm_bo_get_stride(bo),
				gbm_bo_get_handle(bo).u32, &fb)) FAIL("drmModeAddFB failed");
		if (frame == 0) {
			if (drmModeSetCrtc(fd, res->crtcs[0], fb, 0, 0, &conn->connector_id, 1, &mode)) FAIL("SetCrtc failed");
			STEP("frame 0 on screen (modeset)");
		} else {
			int done = 0;
			if (drmModePageFlip(fd, res->crtcs[0], fb, DRM_MODE_PAGE_FLIP_EVENT, &done)) FAIL("page flip failed");
			drmEventContext ev = {.version = 2, .page_flip_handler = flip_done};
			while (!done) {
				struct pollfd p = {.fd = fd, .events = POLLIN};
				poll(&p, 1, 1000);
				drmHandleEvent(fd, &ev);
			}
			STEP("frame %d flipped", frame);
		}
		if (prev_bo) {
			drmModeRmFB(fd, prev_fb);
			gbm_surface_release_buffer(gs, prev_bo);
		}
		prev_bo = bo;
		prev_fb = fb;
	}
	STEP("ok");
	return 0;
}
