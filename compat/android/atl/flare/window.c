/* The app window: an Arcan shmif segment, the frame loop, input, and the
 * activity stack (ported from ATL's GTK version in app/android_app_Activity.c).
 * SPDX-License-Identifier: GPL-3.0-or-later (derived from ATL) */
#include <arcan_shmif.h>
#include <glib.h>
#include <glib-unix.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "flare.h"
#include "util.h"

struct fwin *main_window;
double flare_density = 1.0;

static struct arcan_shmif_cont con;
static bool have_con;
static bool frame_pending, layout_pending;
static guint frame_source;
static long frames;

static long now_ms(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec * 1000L + ts.tv_nsec / 1000000;
}

/* ---------------- views attached to the window ---------------- */

static void set_attached(JNIEnv *env, struct fw *w, bool on)
{
	jobject v = (*env)->NewLocalRef(env, w->view);
	if (v) {
		if (on)
			J__View__onAttachedToWindow(env, v);
		else
			J__View__onDetachedFromWindow(env, v);
		if ((*env)->ExceptionCheck(env))
			{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
		(*env)->DeleteLocalRef(env, v);
	}
	for (int i = 0; i < w->nkids; i++)
		set_attached(env, w->kids[i], on);
}

void flare_show_view(JNIEnv *env, struct fw *root)
{
	if (!main_window || main_window->root == root)
		return;
	if (main_window->root)
		set_attached(env, main_window->root, false);
	main_window->root = root;
	if (root)
		set_attached(env, root, true);
	flare_request_layout();
}

/* ---------------- frames ---------------- */

static gboolean frame_cb(gpointer data);

static void schedule(void)
{
	if (!frame_source)
		frame_source = g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, frame_cb, NULL, NULL);
}

void flare_request_frame(void)
{
	frame_pending = true;
	schedule();
}

void flare_request_layout(void)
{
	layout_pending = true;
	frame_pending = true;
	schedule();
}

static void do_layout(JNIEnv *env, jobject root)
{
	int w = main_window->width, h = main_window->height;
	J__View__measure(env, root, w | MEASURE_SPEC_EXACTLY, h | MEASURE_SPEC_EXACTLY);
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
	jclass vc = _CACHED_CLASS(View);
	static jmethodID layout;
	if (!layout)
		layout = (*env)->GetMethodID(env, vc, "layout", "(IIII)V");
	(*env)->CallVoidMethod(env, root, layout, 0, 0, w, h);
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
}

static void render(JNIEnv *env)
{
	struct fw *root = main_window->root;
	jobject rv = root ? (*env)->NewLocalRef(env, root->view) : NULL;
	for (int pass = 0; rv && layout_pending && pass < 3; pass++) {
		layout_pending = false;
		do_layout(env, rv);
	}
	layout_pending = false;
	frame_pending = false;
	cairo_surface_t *s;
	if (have_con)
		s = cairo_image_surface_create_for_data((unsigned char *)con.vidp, CAIRO_FORMAT_ARGB32, con.w, con.h, con.stride);
	else
		s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, main_window->width, main_window->height);
	struct fcanvas c = {.cr = cairo_create(s)};
	cairo_set_source_rgb(c.cr, 1, 1, 1);
	cairo_paint(c.cr);
	if (rv) {
		jobject canvas = J_new__GskCanvas(env, _INTPTR(&c));
		c.jcanvas = canvas;
		J__View__draw(env, rv, canvas);
		if ((*env)->ExceptionCheck(env))
			{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
		(*env)->DeleteLocalRef(env, canvas);
	}
	cairo_destroy(c.cr);
	cairo_surface_flush(s);
	/* ATL_SNAPSHOT=file.png: save every frame (for headless runs and tests) */
	const char *snap = getenv("ATL_SNAPSHOT");
	if (snap)
		cairo_surface_write_to_png(s, snap);
	cairo_surface_destroy(s);
	if (have_con) {
		/* Arcan's GLES2 build uploads shmif buffers as RGBA bytes (GLES
		 * has no BGRA), while cairo draws native-endian ARGB: swap red
		 * and blue in place */
		for (size_t y = 0; y < con.h; y++) {
			uint32_t *row = (uint32_t *)((uint8_t *)con.vidp + y * con.stride);
			for (size_t x = 0; x < con.w; x++) {
				uint32_t p = row[x];
				row[x] = (p & 0xff00ff00) | ((p >> 16) & 0xff) | ((p & 0xff) << 16);
			}
		}
		arcan_shmif_signal(&con, SHMIF_SIGVID);
	}
	if (rv)
		(*env)->DeleteLocalRef(env, rv);
	if (frames++ == 0)
		fprintf(stderr, "atl: first frame drawn (%dx%d)\n", main_window->width, main_window->height);
}

static gboolean frame_cb(gpointer data)
{
	frame_source = 0;
	if (frame_pending || layout_pending) {
		JNIEnv *env = get_jni_env();
		long t0 = now_ms();
		render(env);
		if (getenv("ATL_FRAME_TIMES"))
			fprintf(stderr, "atl: frame %ld in %ld ms\n", frames, now_ms() - t0);
	}
	return G_SOURCE_REMOVE;
}

/* ---------------- input ---------------- */

static jobject current_activity(void);

#define ACTION_DOWN 0
#define ACTION_UP   1
#define ACTION_MOVE 2
#define SOURCE_TOUCHSCREEN 0x1002

static struct fw *press_target; /* a natively backed widget under the finger */
static int down_x, down_y;
static bool finger_down;

static void set_pressed(JNIEnv *env, struct fw *w, bool on)
{
	w->pressed = on;
	jobject v = (*env)->NewLocalRef(env, w->view);
	if (!v)
		return;
	static jmethodID m;
	if (!m)
		m = (*env)->GetMethodID(env, _CACHED_CLASS(View), "setPressed", "(Z)V");
	(*env)->CallVoidMethod(env, v, m, (jboolean)on);
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
	(*env)->DeleteLocalRef(env, v);
	flare_request_frame();
}

static bool natively_clickable(struct fw *w)
{
	return w->kind == FW_BUTTON || w->kind == FW_EDIT || w->kind == FW_IMAGE;
}

static void touch(JNIEnv *env, int action, int x, int y)
{
	struct fw *root = main_window ? main_window->root : NULL;
	if (!root)
		return;
	jobject rv = (*env)->NewLocalRef(env, root->view);
	if (!rv)
		return;
	if (action == ACTION_DOWN) {
		int lx, ly;
		struct fw *hit = fw_hit(root, x, y, &lx, &ly);
		while (hit && !natively_clickable(hit))
			hit = hit->parent;
		press_target = hit;
		down_x = x;
		down_y = y;
		if (hit && hit->kind != FW_EDIT)
			set_pressed(env, hit, true);
	}
	jobject ev = J_new__MotionEvent__single(env, SOURCE_TOUCHSCREEN, action, (jlong)now_ms(), (float)x, (float)y, (float)x, (float)y);
	J__View__dispatchTouchEvent(env, rv, ev);
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
	(*env)->DeleteLocalRef(env, ev);
	if (action == ACTION_UP && press_target) {
		struct fw *t = press_target;
		press_target = NULL;
		int lx, ly;
		struct fw *hit = fw_hit(root, x, y, &lx, &ly);
		while (hit && hit != t)
			hit = hit->parent;
		if (t->kind != FW_EDIT)
			set_pressed(env, t, false);
		if (hit == t) {
			if (t->kind == FW_EDIT) {
				focused_edit = t;
				flare_request_frame();
			}
			jobject v = (*env)->NewLocalRef(env, t->view);
			if (v) {
				J__View__performClick(env, v);
				if ((*env)->ExceptionCheck(env))
					{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
				(*env)->DeleteLocalRef(env, v);
			}
		}
	}
	(*env)->DeleteLocalRef(env, rv);
}

static void back_pressed(JNIEnv *env)
{
	jobject a = current_activity();
	if (!a)
		return;
	J__Activity__onBackPressed(env, a);
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
}

static void key(JNIEnv *env, arcan_ioevent *io)
{
	if (!io->input.translated.active)
		return;
	uint32_t sym = io->input.translated.keysym;
	if (sym == 27) { /* Escape: Android's back key */
		back_pressed(env);
		return;
	}
	if (!focused_edit)
		return;
	if (sym == 8)
		fw_edit_key(env, focused_edit, NULL, 0xff08);
	else if (sym == 13)
		fw_edit_key(env, focused_edit, NULL, 0xff0d);
	else if (io->input.translated.utf8[0] >= 0x20 || io->input.translated.utf8[0] & 0x80)
		fw_edit_key(env, focused_edit, (const char *)io->input.translated.utf8, 0);
}

static int mouse_x, mouse_y;

static void io_event(JNIEnv *env, arcan_ioevent *io)
{
	if (io->datatype == EVENT_IDATATYPE_TOUCH) {
		int x = io->input.touch.x, y = io->input.touch.y;
		if (io->input.touch.active && !finger_down) {
			finger_down = true;
			touch(env, ACTION_DOWN, x, y);
		} else if (io->input.touch.active) {
			touch(env, ACTION_MOVE, x, y);
		} else if (finger_down) {
			finger_down = false;
			touch(env, ACTION_UP, x, y);
		}
	} else if (io->datatype == EVENT_IDATATYPE_ANALOG && io->devkind == EVENT_IDEVKIND_MOUSE) {
		if (io->input.analog.gotrel)
			return;
		if (io->subid == 0)
			mouse_x = io->input.analog.axisval[0];
		else if (io->subid == 1)
			mouse_y = io->input.analog.axisval[0];
		else if (io->subid == 2 && io->input.analog.nvalues >= 3) {
			mouse_x = io->input.analog.axisval[0];
			mouse_y = io->input.analog.axisval[2];
		}
		if (finger_down)
			touch(env, ACTION_MOVE, mouse_x, mouse_y);
	} else if (io->datatype == EVENT_IDATATYPE_DIGITAL && io->devkind == EVENT_IDEVKIND_MOUSE) {
		if (io->subid != 1) /* MBTN_LEFT_IND */
			return;
		if (io->input.digital.active && !finger_down) {
			finger_down = true;
			touch(env, ACTION_DOWN, mouse_x, mouse_y);
		} else if (!io->input.digital.active && finger_down) {
			finger_down = false;
			touch(env, ACTION_UP, mouse_x, mouse_y);
		}
	} else if (io->datatype == EVENT_IDATATYPE_TRANSLATED) {
		key(env, io);
	}
}

static void resize(int w, int h)
{
	if (w <= 0 || h <= 0 || (w == (int)con.w && h == (int)con.h))
		return;
	if (!arcan_shmif_resize(&con, w, h))
		return;
	main_window->width = con.w;
	main_window->height = con.h;
	JNIEnv *env = get_jni_env();
	jclass dc = _CACHED_CLASS(Display);
	_SET_STATIC_INT_FIELD(dc, "window_width", main_window->width);
	_SET_STATIC_INT_FIELD(dc, "window_height", main_window->height);
	flare_request_layout();
}

static gboolean shmif_cb(gint fd, GIOCondition cond, gpointer data)
{
	JNIEnv *env = get_jni_env();
	struct arcan_event ev;
	int r;
	while ((r = arcan_shmif_poll(&con, &ev)) > 0) {
		if (ev.category == EVENT_IO) {
			io_event(env, &ev.io);
		} else if (ev.category == EVENT_TARGET) {
			switch (ev.tgt.kind) {
				case TARGET_COMMAND_DISPLAYHINT:
					resize(ev.tgt.ioevs[0].iv, ev.tgt.ioevs[1].iv);
					break;
				case TARGET_COMMAND_EXIT:
					fprintf(stderr, "atl: window closed\n");
					exit(0);
				default:
					break;
			}
		}
	}
	if (r < 0) {
		fprintf(stderr, "atl: display server connection lost\n");
		exit(0);
	}
	return G_SOURCE_CONTINUE;
}

int flare_window_init(int *width, int *height, const char *title)
{
	main_window = calloc(1, sizeof(*main_window));
	const char *d = getenv("ATL_DENSITY");
	if (d)
		flare_density = atof(d);
	if (getenv("ARCAN_CONNPATH")) {
		con = arcan_shmif_open(SEGID_APPLICATION, 0, NULL);
		have_con = con.addr != NULL;
	}
	if (!have_con) {
		fprintf(stderr, "atl: no Arcan connection (ARCAN_CONNPATH); running headless\n");
		main_window->width = *width;
		main_window->height = *height;
		return 0;
	}
	if (*width > 0 && *height > 0)
		arcan_shmif_resize(&con, *width, *height);
	*width = main_window->width = con.w;
	*height = main_window->height = con.h;
	struct arcan_event ev = {.category = EVENT_EXTERNAL, .ext.kind = ARCAN_EVENT(IDENT)};
	snprintf((char *)ev.ext.message.data, sizeof(ev.ext.message.data), "%s", title ? title : "android");
	arcan_shmif_enqueue(&con, &ev);
	g_unix_fd_add(con.epipe, G_IO_IN, shmif_cb, NULL);
	fprintf(stderr, "atl: Arcan window %dx%d\n", *width, *height);
	return 1;
}

/* ---------------- activities ---------------- */

static GList *backlog;
static jobject activity_now;

static jobject current_activity(void)
{
	return activity_now;
}

static struct fw *decor_of(JNIEnv *env, jobject activity)
{
	jobject win = _GET_OBJ_FIELD(activity, "window", "Landroid/view/Window;");
	if (!win)
		return NULL;
	jobject decor = _GET_OBJ_FIELD(win, "decorView", "Landroid/view/ViewGroup;");
	struct fw *w = fw_of(env, decor);
	if (main_window && !main_window->jwindow)
		main_window->jwindow = (*env)->NewGlobalRef(env, win);
	return w;
}

#define CHECK()                                    \
	do {                                       \
		if ((*env)->ExceptionCheck(env))   \
			{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); } \
	} while (0)

static void activity_close(JNIEnv *env, jobject a)
{
	CHECK();
	J__Activity__dispatchActivityPreDestroyed(env, a);
	CHECK();
	J__Activity__onDestroy(env, a);
	CHECK();
}

static void activity_unfocus(JNIEnv *env, jobject a)
{
	if (!_GET_BOOL_FIELD(a, "paused")) {
		J__Activity__dispatchActivityPrePaused(env, a);
		CHECK();
		J__Activity__onPause(env, a);
		CHECK();
	}
	J__Activity__dispatchActivityPreStopped(env, a);
	CHECK();
	J__Activity__onStop(env, a);
	CHECK();
	J__Activity__onWindowFocusChanged(env, a, false);
	CHECK();
}

static void activity_focus(JNIEnv *env, jobject a)
{
	if (_GET_BOOL_FIELD(a, "finishing"))
		return;
	J__Activity__onStart(env, a);
	CHECK();
	J__Activity__dispatchActivityPostStarted(env, a);
	CHECK();
	if (_GET_BOOL_FIELD(a, "finishing"))
		return;
	flare_show_view(env, decor_of(env, a));
	J__Activity__onResume(env, a);
	CHECK();
	if (_GET_BOOL_FIELD(a, "finishing"))
		return;
	J__Activity__onPostResume(env, a);
	CHECK();
	J__Activity__dispatchActivityPostResumed(env, a);
	CHECK();
	if (_GET_BOOL_FIELD(a, "finishing"))
		return;
	J__Activity__onWindowFocusChanged(env, a, true);
	CHECK();
	jstring name = J__Class__getName(env, (*env)->GetObjectClass(env, a));
	const char *n = (*env)->GetStringUTFChars(env, name, NULL);
	fprintf(stderr, "atl: %s resumed\n", n);
	(*env)->ReleaseStringUTFChars(env, name, n);
}

static void update_current(JNIEnv *env)
{
	jobject next = backlog ? backlog->data : NULL;
	if (activity_now != next) {
		if (activity_now)
			activity_unfocus(env, activity_now);
		activity_now = NULL;
		if (next)
			activity_focus(env, next);
		if (next && _GET_BOOL_FIELD(next, "finishing"))
			return;
		activity_now = next;
	}
}

void activity_start(JNIEnv *env, jobject a)
{
	if (activity_now)
		activity_unfocus(env, activity_now);
	activity_now = NULL;
	J__Activity__onCreate(env, a, NULL);
	CHECK();
	J__Activity__dispatchActivityPostCreated(env, a);
	CHECK();
	if (_GET_BOOL_FIELD(a, "finishing"))
		return;
	J__Activity__onPostCreate(env, a, NULL);
	CHECK();
	backlog = g_list_prepend(backlog, _REF(a));
	update_current(env);
}

void activity_window_ready(void) {}

void activity_close_all(void)
{
	JNIEnv *env = get_jni_env();
	GList *all = backlog;
	backlog = NULL;
	update_current(env);
	for (GList *l = all; l; l = l->next) {
		activity_close(env, l->data);
		_UNREF(l->data);
	}
	g_list_free(all);
}

void current_activity_back_pressed(void)
{
	back_pressed(get_jni_env());
}

static gboolean quit_cb(gpointer data)
{
	if (!backlog) {
		fprintf(stderr, "atl: last activity finished\n");
		exit(0);
	}
	return G_SOURCE_REMOVE;
}

JNIEXPORT void JNICALL Java_android_app_Activity_nativeFinish(JNIEnv *env, jobject self, jlong window)
{
	jobject removed = NULL;
	for (GList *l = backlog; l; l = l->next)
		if ((*env)->IsSameObject(env, self, l->data)) {
			removed = l->data;
			backlog = g_list_delete_link(backlog, l);
			break;
		}
	update_current(env);
	if (removed) {
		activity_close(env, removed);
		_UNREF(removed);
	}
	if (!backlog && window)
		g_idle_add(quit_cb, NULL);
}

JNIEXPORT void JNICALL Java_android_app_Activity_nativeStartActivity(JNIEnv *env, jclass c, jobject a)
{
	activity_start(env, a);
}

JNIEXPORT jboolean JNICALL Java_android_app_Activity_nativeResumeActivity(JNIEnv *env, jclass c, jclass cls, jobject intent)
{
	GList *close = NULL;
	jboolean found = false;
	for (GList *l = backlog; l; l = l->next)
		if ((*env)->IsSameObject(env, cls, _CLASS(l->data))) {
			if (l != backlog) {
				close = backlog;
				backlog = l;
				l->prev->next = NULL;
				l->prev = NULL;
			}
			J__Activity__onNewIntent(env, l->data, intent);
			CHECK();
			found = true;
			break;
		}
	update_current(env);
	for (GList *l = close; l; l = l->next) {
		activity_close(env, l->data);
		_UNREF(l->data);
	}
	g_list_free(close);
	return found;
}

JNIEXPORT jboolean JNICALL Java_android_app_Activity_isInMultiWindowMode(JNIEnv *env, jobject self) { return false; }
JNIEXPORT jboolean JNICALL Java_android_app_Activity_isTaskRoot(JNIEnv *env, jobject self) { return g_list_length(backlog) <= 1; }

/* ---------------- Window ---------------- */

JNIEXPORT void JNICALL Java_android_view_Window_set_1title(JNIEnv *env, jobject self, jlong w, jstring title)
{
	if (!have_con || !title)
		return;
	const char *t = (*env)->GetStringUTFChars(env, title, NULL);
	struct arcan_event ev = {.category = EVENT_EXTERNAL, .ext.kind = ARCAN_EVENT(IDENT)};
	snprintf((char *)ev.ext.message.data, sizeof(ev.ext.message.data), "%s", t);
	(*env)->ReleaseStringUTFChars(env, title, t);
	arcan_shmif_enqueue(&con, &ev);
}

JNIEXPORT void JNICALL Java_android_view_Window_set_1jobject(JNIEnv *env, jclass c, jlong w, jobject win)
{
	if (main_window && !main_window->jwindow)
		main_window->jwindow = (*env)->NewGlobalRef(env, win);
}

JNIEXPORT void JNICALL Java_android_view_Window_set_1layout(JNIEnv *env, jobject self, jlong w, jint width, jint height) {}
JNIEXPORT void JNICALL Java_android_view_Window_remove_1gtk_1background(JNIEnv *env, jobject self, jlong w) {}

/* ---------------- WindowManager: popups and overlays share the window ---------------- */

JNIEXPORT void JNICALL Java_android_view_WindowManagerImpl_native_1addView(JNIEnv *env, jclass c, jlong p, jint type, jint x, jint y, jint w, jint h)
{
	fprintf(stderr, "atl: WindowManager.addView (overlay windows are not shown yet)\n");
}

/* ---------------- Configuration: the window decides ---------------- */

JNIEXPORT void JNICALL Java_android_content_Context_native_1updateConfig(JNIEnv *env, jclass c, jobject config)
{
	int w = main_window ? main_window->width : 480, h = main_window ? main_window->height : 800;
	int wdp = w / flare_density, hdp = h / flare_density;
	_SET_INT_FIELD(config, "screenWidthDp", wdp);
	_SET_INT_FIELD(config, "screenHeightDp", hdp);
	_SET_INT_FIELD(config, "smallestScreenWidthDp", wdp < hdp ? wdp : hdp);
	_SET_INT_FIELD(config, "screenLayout", w >= 800 && h >= 800 ? 0x03 /* LARGE */ : 0x02 /* NORMAL */);
	_SET_INT_FIELD(config, "orientation", h >= w ? 1 /* PORTRAIT */ : 2 /* LANDSCAPE */);
	_SET_INT_FIELD(config, "densityDpi", (int)(160 * flare_density));
	_SET_INT_FIELD(config, "touchscreen", 3 /* FINGER */);
	_SET_INT_FIELD(config, "uiMode", 0x11 /* NORMAL, NIGHT_NO */);
}

/* ---------------- main loop ---------------- */

void flare_run(JNIEnv *env)
{
	GMainLoop *loop = g_main_loop_new(NULL, FALSE);
	flare_request_layout();
	g_main_loop_run(loop);
}
