/* Flare's native backend for Android Translation Layer.
 *
 * Upstream ATL implements the native half of android.view and
 * android.graphics with GTK4 widgets and GSK render nodes. On Flare the
 * same JNI entry points are implemented here with no toolkit at all:
 *
 *   - every View owns a small native node (struct fw) holding what GTK's
 *     widget held: parent and children, layout box, background, padding,
 *     visibility and, for the text and image widgets, their content;
 *   - layout stays in Java (measure/onMeasure, layout/onLayout); native
 *     code only stores the boxes and measures text and images;
 *   - drawing walks the tree through View.draw(canvas), so app views that
 *     override onDraw and framework views take the same path. A canvas
 *     handle (the "snapshot" of GskCanvas) is a cairo context;
 *   - the window is an Arcan shmif segment; its input becomes MotionEvent
 *     and KeyEvent, dispatched in Java by the decor view.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later (derived from ATL)
 */
#ifndef FLARE_H
#define FLARE_H

#include <cairo.h>
#include <jni.h>
#include <pango/pangocairo.h>
#include <stdbool.h>
#include <stdint.h>

#include "defines.h"
#include "upcalls.h"
#include "util.h"

#define MEASURE_SPEC_MASK    (0x3 << 30)
#define MEASURE_SPEC_EXACTLY (1 << 30)
#define MEASURE_SPEC_AT_MOST (2 << 30)

/* ---- paintables: what a Drawable or Bitmap draws ---- */

enum { FP_JAVA, FP_SURFACE, FP_NINEPATCH };

struct fpaintable {
	int refs;
	int kind;
	jobject drawable;          /* FP_JAVA: weak ref to the Java Drawable */
	cairo_surface_t *surface;  /* FP_SURFACE, FP_NINEPATCH */
	int xdiv[2], ydiv[2];      /* FP_NINEPATCH: one stretchable span per axis */
	uint32_t tint;
};

struct fpaintable *fp_new(int kind);
struct fpaintable *fp_ref(struct fpaintable *p);
void fp_unref(struct fpaintable *p);
void fp_draw(JNIEnv *env, struct fpaintable *p, cairo_t *cr, double w, double h);
int fp_width(JNIEnv *env, struct fpaintable *p);
int fp_height(JNIEnv *env, struct fpaintable *p);

/* ---- canvases ---- */

struct fcanvas {
	cairo_t *cr;
	cairo_surface_t *surface; /* the target (owned for bitmap/node canvases) */
	jobject jcanvas;          /* the GskCanvas wrapping this, for tree walks */
};

struct fcanvas *fc_new(cairo_surface_t *target);
void fc_free(JNIEnv *env, struct fcanvas *c);

/* ---- Paint ---- */

struct fpaint {
	double r, g, b, a;
	bool fill, stroke;
	double stroke_width;
	int cap, join;
	double text_size;
	int text_align; /* 0 left, 1 center, 2 right */
	bool bold, italic;
	bool filter;
	double fr, fg, fb, fa; /* colour filter: the colour replaces the source */
};

void paint_source(cairo_t *cr, struct fpaint *p);
PangoLayout *text_layout(cairo_t *cr, const char *text, double size_px, bool bold, bool italic);

/* ---- Path ---- */

enum { PO_MOVE, PO_LINE, PO_CUBIC, PO_CLOSE };
struct pop {
	int op;
	float p[6];
};
struct fpath {
	int refs;
	int n, cap;
	struct pop *ops;
	float cx, cy, sx, sy; /* current point, subpath start */
};
void path_replay(cairo_t *cr, struct fpath *p);

/* ---- native view nodes ---- */

enum { FW_VIEW, FW_GROUP, FW_TEXT, FW_BUTTON, FW_EDIT, FW_IMAGE, FW_SCROLL };

struct fw {
	jobject view; /* weak global ref */
	struct fw *parent;
	struct fw **kids;
	int nkids, cap;
	int kind;
	int x, y, w, h;
	int lp_w, lp_h;
	int pad[4];
	bool visible, focused;
	float alpha;
	bool has_bg_color;
	uint32_t bg_color;
	struct fpaintable *bg;
	/* text widgets */
	char *text, *hint;
	uint32_t color;
	float text_size;
	bool bold, italic, upper, markup, no_border;
	int align; /* -1 default, 0 left, 1 center, 2 right */
	struct fpaintable *compound[4];
	jobject watchers[8]; /* EditText TextWatchers (global refs) */
	int nwatchers;
	jobject editor_action;
	bool pressed;
	/* image widgets */
	struct fpaintable *image;
	int scale_type;
	struct fwin *win; /* for a decor view: the window it is the root of */
};

struct fw *fw_of(JNIEnv *env, jobject view);
struct fw *fw_new(JNIEnv *env, jobject view, int kind);
void fw_text_size(struct fw *w, int for_width, int *out_w, int *out_h);
struct fw *fw_hit(struct fw *root, int x, int y, int *lx, int *ly);
void fw_edit_key(JNIEnv *env, struct fw *w, const char *utf8, int keysym);
extern struct fw *focused_edit;

/* ---- windows ---- */

struct fwin {
	jobject jwindow;   /* android.view.Window */
	struct fw *root;   /* the decor view shown in this window */
	int width, height; /* in pixels */
};

extern struct fwin *main_window;
extern double flare_density; /* display density, 1.0 = 160 dpi */
void flare_request_frame(void);
void flare_request_layout(void);
void flare_show_view(JNIEnv *env, struct fw *root);
int flare_window_init(int *width, int *height, const char *title);
void flare_run(JNIEnv *env);

static inline void color_split(uint32_t c, double *r, double *g, double *b, double *a)
{
	*a = ((c >> 24) & 0xff) / 255.0;
	*r = ((c >> 16) & 0xff) / 255.0;
	*g = ((c >> 8) & 0xff) / 255.0;
	*b = (c & 0xff) / 255.0;
}

#endif
