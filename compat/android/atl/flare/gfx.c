/* android.graphics on cairo and Pango: Paint, Path, Matrix, Canvas,
 * Bitmap, Drawable and text Layout.
 * SPDX-License-Identifier: GPL-3.0-or-later (derived from ATL) */
#include <arpa/inet.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "flare.h"

/* ---------------- paintables ---------------- */

struct fpaintable *fp_new(int kind)
{
	struct fpaintable *p = calloc(1, sizeof(*p));
	p->refs = 1;
	p->kind = kind;
	return p;
}

struct fpaintable *fp_ref(struct fpaintable *p)
{
	if (p)
		p->refs++;
	return p;
}

void fp_unref(struct fpaintable *p)
{
	if (!p || --p->refs > 0)
		return;
	if (p->surface)
		cairo_surface_destroy(p->surface);
	if (p->drawable) {
		JNIEnv *env = get_jni_env();
		(*env)->DeleteWeakGlobalRef(env, p->drawable);
	}
	free(p);
}

static void draw_ninepatch(cairo_t *cr, struct fpaintable *p, double w, double h)
{
	cairo_surface_t *s = p->surface;
	int sw = cairo_image_surface_get_width(s), sh = cairo_image_surface_get_height(s);
	/* columns and rows: fixed, stretched, fixed */
	double sx[4] = {0, p->xdiv[0], p->xdiv[1], sw}, sy[4] = {0, p->ydiv[0], p->ydiv[1], sh};
	double fixw = sx[1] + (sx[3] - sx[2]), fixh = sy[1] + (sy[3] - sy[2]);
	double dx[4] = {0, sx[1], w - (sx[3] - sx[2]), w}, dy[4] = {0, sy[1], h - (sy[3] - sy[2]), h};
	if (w < fixw)
		dx[1] = dx[2] = w * sx[1] / (fixw ? fixw : 1);
	if (h < fixh)
		dy[1] = dy[2] = h * sy[1] / (fixh ? fixh : 1);
	for (int i = 0; i < 3; i++)
		for (int j = 0; j < 3; j++) {
			double cw = sx[i + 1] - sx[i], ch = sy[j + 1] - sy[j];
			double dw = dx[i + 1] - dx[i], dh = dy[j + 1] - dy[j];
			if (cw <= 0 || ch <= 0 || dw <= 0 || dh <= 0)
				continue;
			cairo_save(cr);
			cairo_rectangle(cr, dx[i], dy[j], dw, dh);
			cairo_clip(cr);
			cairo_translate(cr, dx[i], dy[j]);
			cairo_scale(cr, dw / cw, dh / ch);
			cairo_set_source_surface(cr, s, -sx[i], -sy[j]);
			cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BILINEAR);
			cairo_paint(cr);
			cairo_restore(cr);
		}
}

void fp_draw(JNIEnv *env, struct fpaintable *p, cairo_t *cr, double w, double h)
{
	if (!p || w <= 0 || h <= 0)
		return;
	if (p->kind == FP_JAVA) {
		jobject d = (*env)->NewLocalRef(env, p->drawable);
		if (!d)
			return;
		struct fcanvas c = {.cr = cr};
		jobject canvas = J_new__GskCanvas(env, _INTPTR(&c));
		c.jcanvas = canvas;
		J__Drawable__setBounds(env, d, 0, 0, (int)w, (int)h);
		J__Drawable__draw(env, d, canvas);
		if ((*env)->ExceptionCheck(env))
			{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
		(*env)->DeleteLocalRef(env, canvas);
		(*env)->DeleteLocalRef(env, d);
		return;
	}
	if (!p->surface)
		return;
	cairo_save(cr);
	if (p->tint) {
		cairo_push_group(cr);
	}
	if (p->kind == FP_NINEPATCH) {
		draw_ninepatch(cr, p, w, h);
	} else {
		int sw = cairo_image_surface_get_width(p->surface), sh = cairo_image_surface_get_height(p->surface);
		cairo_rectangle(cr, 0, 0, w, h);
		cairo_clip(cr);
		cairo_scale(cr, w / sw, h / sh);
		cairo_set_source_surface(cr, p->surface, 0, 0);
		cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BILINEAR);
		cairo_paint(cr);
	}
	if (p->tint) {
		cairo_pattern_t *mask = cairo_pop_group(cr);
		double r, g, b, a;
		color_split(p->tint, &r, &g, &b, &a);
		cairo_set_source_rgba(cr, r, g, b, a);
		cairo_mask(cr, mask);
		cairo_pattern_destroy(mask);
	}
	cairo_restore(cr);
}

int fp_width(JNIEnv *env, struct fpaintable *p)
{
	if (!p)
		return 0;
	if (p->kind == FP_JAVA) {
		jobject d = (*env)->NewLocalRef(env, p->drawable);
		int v = d ? J__Drawable__getIntrinsicWidth(env, d) : 0;
		return v > 0 ? v : 0;
	}
	return p->surface ? cairo_image_surface_get_width(p->surface) : 0;
}

int fp_height(JNIEnv *env, struct fpaintable *p)
{
	if (!p)
		return 0;
	if (p->kind == FP_JAVA) {
		jobject d = (*env)->NewLocalRef(env, p->drawable);
		int v = d ? J__Drawable__getIntrinsicHeight(env, d) : 0;
		return v > 0 ? v : 0;
	}
	return p->surface ? cairo_image_surface_get_height(p->surface) : 0;
}

/* ---------------- canvases ---------------- */

struct fcanvas *fc_new(cairo_surface_t *target)
{
	struct fcanvas *c = calloc(1, sizeof(*c));
	c->surface = target;
	c->cr = cairo_create(target);
	return c;
}

void fc_free(JNIEnv *env, struct fcanvas *c)
{
	if (!c)
		return;
	if (c->jcanvas)
		(*env)->DeleteGlobalRef(env, c->jcanvas);
	cairo_destroy(c->cr);
	if (c->surface)
		cairo_surface_destroy(c->surface);
	free(c);
}

#define CR(ptr) (((struct fcanvas *)_PTR(ptr))->cr)
#define PAINT(ptr) ((struct fpaint *)_PTR(ptr))

/* ---------------- Paint ---------------- */

void paint_source(cairo_t *cr, struct fpaint *p)
{
	if (p->filter)
		cairo_set_source_rgba(cr, p->fr, p->fg, p->fb, p->fa * p->a);
	else
		cairo_set_source_rgba(cr, p->r, p->g, p->b, p->a);
}

static PangoContext *pango_ctx;

PangoLayout *text_layout(cairo_t *cr, const char *text, double size_px, bool bold, bool italic)
{
	PangoLayout *l;
	if (cr) {
		l = pango_cairo_create_layout(cr);
	} else {
		if (!pango_ctx)
			pango_ctx = pango_font_map_create_context(pango_cairo_font_map_get_default());
		l = pango_layout_new(pango_ctx);
	}
	PangoFontDescription *fd = pango_font_description_from_string("DejaVu Sans");
	pango_font_description_set_absolute_size(fd, (size_px > 0 ? size_px : 14) * PANGO_SCALE);
	if (bold)
		pango_font_description_set_weight(fd, PANGO_WEIGHT_BOLD);
	if (italic)
		pango_font_description_set_style(fd, PANGO_STYLE_ITALIC);
	pango_layout_set_font_description(l, fd);
	pango_font_description_free(fd);
	pango_layout_set_text(l, text ? text : "", -1);
	return l;
}

JNIEXPORT jlong JNICALL Java_android_graphics_Paint_native_1create(JNIEnv *env, jclass clazz)
{
	struct fpaint *p = calloc(1, sizeof(*p));
	p->a = 1;
	p->fill = true;
	p->stroke_width = 1;
	p->text_size = 14 * flare_density;
	return _INTPTR(p);
}

JNIEXPORT jlong JNICALL Java_android_graphics_Paint_native_1clone(JNIEnv *env, jclass clazz, jlong ptr)
{
	struct fpaint *p = malloc(sizeof(*p));
	*p = *PAINT(ptr);
	return _INTPTR(p);
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1recycle(JNIEnv *env, jclass clazz, jlong ptr)
{
	free(PAINT(ptr));
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1color(JNIEnv *env, jclass clazz, jlong ptr, jint color)
{
	struct fpaint *p = PAINT(ptr);
	color_split(color, &p->r, &p->g, &p->b, &p->a);
}

JNIEXPORT jint JNICALL Java_android_graphics_Paint_native_1get_1color(JNIEnv *env, jclass clazz, jlong ptr)
{
	struct fpaint *p = PAINT(ptr);
	return ((int)lround(p->a * 255) << 24) | ((int)lround(p->r * 255) << 16) | ((int)lround(p->g * 255) << 8) | (int)lround(p->b * 255);
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1alpha(JNIEnv *env, jclass clazz, jlong ptr, jint a)
{
	PAINT(ptr)->a = (a & 0xff) / 255.0;
}

JNIEXPORT jint JNICALL Java_android_graphics_Paint_native_1get_1alpha(JNIEnv *env, jclass clazz, jlong ptr)
{
	return (jint)lround(PAINT(ptr)->a * 255);
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1style(JNIEnv *env, jclass clazz, jlong ptr, jint style)
{
	struct fpaint *p = PAINT(ptr);
	p->fill = style == 0 || style == 2;
	p->stroke = style == 1 || style == 2;
}

JNIEXPORT jint JNICALL Java_android_graphics_Paint_native_1get_1style(JNIEnv *env, jclass clazz, jlong ptr)
{
	struct fpaint *p = PAINT(ptr);
	return p->fill && p->stroke ? 2 : p->stroke ? 1 : 0;
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1stroke_1width(JNIEnv *env, jclass clazz, jlong ptr, jfloat w)
{
	PAINT(ptr)->stroke_width = w;
}

JNIEXPORT jfloat JNICALL Java_android_graphics_Paint_native_1get_1stroke_1width(JNIEnv *env, jclass clazz, jlong ptr)
{
	return PAINT(ptr)->stroke_width;
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1stroke_1cap(JNIEnv *env, jclass clazz, jlong ptr, jint cap)
{
	PAINT(ptr)->cap = cap;
}

JNIEXPORT jint JNICALL Java_android_graphics_Paint_native_1get_1stroke_1cap(JNIEnv *env, jclass clazz, jlong ptr)
{
	return PAINT(ptr)->cap;
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1stroke_1join(JNIEnv *env, jclass clazz, jlong ptr, jint join)
{
	PAINT(ptr)->join = join;
}

JNIEXPORT jint JNICALL Java_android_graphics_Paint_native_1get_1stroke_1join(JNIEnv *env, jclass clazz, jlong ptr)
{
	return PAINT(ptr)->join;
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1text_1size(JNIEnv *env, jclass clazz, jlong ptr, jfloat size)
{
	PAINT(ptr)->text_size = size;
}

JNIEXPORT jfloat JNICALL Java_android_graphics_Paint_native_1get_1text_1size(JNIEnv *env, jclass clazz, jlong ptr)
{
	return PAINT(ptr)->text_size;
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1color_1filter(JNIEnv *env, jclass clazz, jlong ptr, jint mode, jint color)
{
	struct fpaint *p = PAINT(ptr);
	p->filter = mode != -1;
	color_split(color, &p->fr, &p->fg, &p->fb, &p->fa);
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1set_1text_1align(JNIEnv *env, jclass clazz, jlong ptr, jint align)
{
	PAINT(ptr)->text_align = align;
}

static PangoLayout *paint_layout(JNIEnv *env, struct fpaint *p, jstring text)
{
	const char *s = text ? (*env)->GetStringUTFChars(env, text, NULL) : NULL;
	PangoLayout *l = text_layout(NULL, s, p->text_size, p->bold, p->italic);
	if (s)
		(*env)->ReleaseStringUTFChars(env, text, s);
	return l;
}

JNIEXPORT void JNICALL Java_android_graphics_Paint_native_1get_1text_1bounds(JNIEnv *env, jclass clazz, jlong ptr, jstring text, jobject bounds)
{
	struct fpaint *p = PAINT(ptr);
	PangoLayout *l = paint_layout(env, p, text);
	PangoRectangle ink;
	pango_layout_get_pixel_extents(l, &ink, NULL);
	int base = pango_layout_get_baseline(l) / PANGO_SCALE;
	int x = ink.x;
	if (p->text_align == 1)
		x -= ink.width / 2;
	else if (p->text_align == 2)
		x -= ink.width;
	_SET_INT_FIELD(bounds, "left", x);
	_SET_INT_FIELD(bounds, "top", ink.y - base);
	_SET_INT_FIELD(bounds, "right", x + ink.width);
	_SET_INT_FIELD(bounds, "bottom", ink.y - base + ink.height);
	g_object_unref(l);
}

static void metrics(struct fpaint *p, double *ascent, double *descent, double *height)
{
	PangoLayout *l = text_layout(NULL, "Mg", p->text_size, p->bold, p->italic);
	PangoContext *ctx = pango_layout_get_context(l);
	PangoFontMetrics *m = pango_context_get_metrics(ctx, pango_layout_get_font_description(l), NULL);
	*ascent = (double)pango_font_metrics_get_ascent(m) / PANGO_SCALE;
	*descent = (double)pango_font_metrics_get_descent(m) / PANGO_SCALE;
	*height = (double)pango_font_metrics_get_height(m) / PANGO_SCALE;
	if (*height <= 0)
		*height = *ascent + *descent;
	pango_font_metrics_unref(m);
	g_object_unref(l);
}

JNIEXPORT jfloat JNICALL Java_android_graphics_Paint_native_1get_1font_1metrics(JNIEnv *env, jclass clazz, jlong ptr, jobject fm)
{
	double a, d, h;
	metrics(PAINT(ptr), &a, &d, &h);
	if (fm) {
		_SET_FLOAT_FIELD(fm, "top", -a);
		_SET_FLOAT_FIELD(fm, "ascent", -a);
		_SET_FLOAT_FIELD(fm, "descent", d);
		_SET_FLOAT_FIELD(fm, "bottom", d);
		_SET_FLOAT_FIELD(fm, "leading", h - a - d);
	}
	return h;
}

JNIEXPORT jint JNICALL Java_android_graphics_Paint_native_1get_1font_1metrics_1int(JNIEnv *env, jclass clazz, jlong ptr, jobject fm)
{
	double a, d, h;
	metrics(PAINT(ptr), &a, &d, &h);
	int ia = ceil(a), id = ceil(d), ih = ceil(h);
	if (fm) {
		_SET_INT_FIELD(fm, "top", -ia);
		_SET_INT_FIELD(fm, "ascent", -ia);
		_SET_INT_FIELD(fm, "descent", id);
		_SET_INT_FIELD(fm, "bottom", id);
		_SET_INT_FIELD(fm, "leading", ih - ia - id);
	}
	return ih;
}

/* ---------------- Path ---------------- */

static struct fpath *path_new(void)
{
	struct fpath *p = calloc(1, sizeof(*p));
	p->refs = 1;
	return p;
}

static void path_unref(struct fpath *p)
{
	if (p && --p->refs <= 0) {
		free(p->ops);
		free(p);
	}
}

static void path_push(struct fpath *p, int op, const float *v, int nv)
{
	if (p->n == p->cap) {
		p->cap = p->cap ? p->cap * 2 : 16;
		p->ops = realloc(p->ops, p->cap * sizeof(struct pop));
	}
	struct pop *o = &p->ops[p->n++];
	o->op = op;
	memcpy(o->p, v, nv * sizeof(float));
	if (op == PO_MOVE) {
		p->cx = p->sx = v[0];
		p->cy = p->sy = v[1];
	} else if (op == PO_CLOSE) {
		p->cx = p->sx;
		p->cy = p->sy;
	} else if (nv >= 2) {
		p->cx = v[nv - 2];
		p->cy = v[nv - 1];
	}
}

static void path_append(struct fpath *dst, struct fpath *src, const float *m)
{
	for (int i = 0; i < src->n; i++) {
		struct pop o = src->ops[i];
		if (m)
			for (int k = 0; k < 6; k += 2) {
				float x = o.p[k], y = o.p[k + 1];
				o.p[k] = m[0] * x + m[1] * y + m[2];
				o.p[k + 1] = m[3] * x + m[4] * y + m[5];
			}
		path_push(dst, o.op, o.p, o.op == PO_CUBIC ? 6 : o.op == PO_CLOSE ? 0 : 2);
	}
}

void path_replay(cairo_t *cr, struct fpath *p)
{
	cairo_new_path(cr);
	if (!p)
		return;
	for (int i = 0; i < p->n; i++) {
		struct pop *o = &p->ops[i];
		switch (o->op) {
			case PO_MOVE: cairo_move_to(cr, o->p[0], o->p[1]); break;
			case PO_LINE: cairo_line_to(cr, o->p[0], o->p[1]); break;
			case PO_CUBIC: cairo_curve_to(cr, o->p[0], o->p[1], o->p[2], o->p[3], o->p[4], o->p[5]); break;
			case PO_CLOSE: cairo_close_path(cr); break;
		}
	}
}

/* an elliptical arc as cubic Béziers, angles in degrees, clockwise (y down) */
static void path_arc(struct fpath *p, float l, float t, float r, float b, float start, float sweep, bool move)
{
	double cx = (l + r) / 2, cy = (t + b) / 2, rx = (r - l) / 2, ry = (b - t) / 2;
	double a0 = start * M_PI / 180, total = sweep * M_PI / 180;
	int segs = (int)ceil(fabs(total) / (M_PI / 2));
	if (segs < 1)
		segs = 1;
	double step = total / segs, k = 4.0 / 3.0 * tan(step / 4);
	float pt[2] = {cx + rx * cos(a0), cy + ry * sin(a0)};
	path_push(p, move || !p->n ? PO_MOVE : PO_LINE, pt, 2);
	for (int i = 0; i < segs; i++) {
		double a1 = a0 + step;
		float c[6] = {
			cx + rx * (cos(a0) - k * sin(a0)), cy + ry * (sin(a0) + k * cos(a0)),
			cx + rx * (cos(a1) + k * sin(a1)), cy + ry * (sin(a1) - k * cos(a1)),
			cx + rx * cos(a1), cy + ry * sin(a1)};
		path_push(p, PO_CUBIC, c, 6);
		a0 = a1;
	}
}

#define PATH(ptr) ((struct fpath *)_PTR(ptr))

JNIEXPORT jlong JNICALL Java_android_graphics_Path_native_1create_1builder(JNIEnv *env, jclass c, jlong path, jlong builder)
{
	struct fpath *b = builder ? PATH(builder) : path_new();
	if (path) {
		path_append(b, PATH(path), NULL);
		path_unref(PATH(path));
	}
	return _INTPTR(b);
}

JNIEXPORT jlong JNICALL Java_android_graphics_Path_native_1create_1path(JNIEnv *env, jclass c, jlong builder)
{
	/* the builder's contents move into a new path; the builder restarts empty */
	struct fpath *p = path_new();
	if (builder) {
		struct fpath *b = PATH(builder);
		*p = *b;
		p->refs = 1;
		b->ops = NULL;
		b->n = b->cap = 0;
	}
	return _INTPTR(p);
}

JNIEXPORT jlong JNICALL Java_android_graphics_Path_native_1ref_1path(JNIEnv *env, jclass c, jlong path)
{
	PATH(path)->refs++;
	return path;
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1reset(JNIEnv *env, jclass c, jlong path, jlong builder)
{
	path_unref(PATH(path));
	path_unref(PATH(builder));
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1close(JNIEnv *env, jclass c, jlong b)
{
	path_push(PATH(b), PO_CLOSE, NULL, 0);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1move_1to(JNIEnv *env, jclass c, jlong b, jfloat x, jfloat y)
{
	path_push(PATH(b), PO_MOVE, (float[]){x, y}, 2);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1line_1to(JNIEnv *env, jclass c, jlong b, jfloat x, jfloat y)
{
	struct fpath *p = PATH(b);
	if (!p->n)
		path_push(p, PO_MOVE, (float[]){0, 0}, 2);
	path_push(p, PO_LINE, (float[]){x, y}, 2);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1cubic_1to(JNIEnv *env, jclass c, jlong b, jfloat x1, jfloat y1, jfloat x2, jfloat y2, jfloat x3, jfloat y3)
{
	struct fpath *p = PATH(b);
	if (!p->n)
		path_push(p, PO_MOVE, (float[]){0, 0}, 2);
	path_push(p, PO_CUBIC, (float[]){x1, y1, x2, y2, x3, y3}, 6);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1quad_1to(JNIEnv *env, jclass c, jlong b, jfloat x1, jfloat y1, jfloat x2, jfloat y2)
{
	struct fpath *p = PATH(b);
	if (!p->n)
		path_push(p, PO_MOVE, (float[]){0, 0}, 2);
	float x0 = p->cx, y0 = p->cy;
	path_push(p, PO_CUBIC, (float[]){x0 + 2.f / 3 * (x1 - x0), y0 + 2.f / 3 * (y1 - y0), x2 + 2.f / 3 * (x1 - x2), y2 + 2.f / 3 * (y1 - y2), x2, y2}, 6);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1arc_1to(JNIEnv *env, jclass c, jlong b, jfloat l, jfloat t, jfloat r, jfloat bo, jfloat start, jfloat sweep, jboolean move)
{
	path_arc(PATH(b), l, t, r, bo, start, sweep, move);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1rel_1move_1to(JNIEnv *env, jclass c, jlong b, jfloat x, jfloat y)
{
	struct fpath *p = PATH(b);
	path_push(p, PO_MOVE, (float[]){p->cx + x, p->cy + y}, 2);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1rel_1line_1to(JNIEnv *env, jclass c, jlong b, jfloat x, jfloat y)
{
	struct fpath *p = PATH(b);
	Java_android_graphics_Path_native_1line_1to(env, c, b, p->cx + x, p->cy + y);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1rel_1cubic_1to(JNIEnv *env, jclass c, jlong b, jfloat x1, jfloat y1, jfloat x2, jfloat y2, jfloat x3, jfloat y3)
{
	struct fpath *p = PATH(b);
	float x = p->cx, y = p->cy;
	Java_android_graphics_Path_native_1cubic_1to(env, c, b, x + x1, y + y1, x + x2, y + y2, x + x3, y + y3);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1rel_1quad_1to(JNIEnv *env, jclass c, jlong b, jfloat x1, jfloat y1, jfloat x2, jfloat y2)
{
	struct fpath *p = PATH(b);
	float x = p->cx, y = p->cy;
	Java_android_graphics_Path_native_1quad_1to(env, c, b, x + x1, y + y1, x + x2, y + y2);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1add_1arc(JNIEnv *env, jclass c, jlong b, jfloat l, jfloat t, jfloat r, jfloat bo, jfloat start, jfloat sweep)
{
	path_arc(PATH(b), l, t, r, bo, start, sweep, true);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1add_1path(JNIEnv *env, jclass c, jlong b, jlong path, jlong matrix)
{
	path_append(PATH(b), PATH(path), matrix ? (float *)_PTR(matrix) : NULL);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1add_1rect(JNIEnv *env, jclass c, jlong b, jfloat l, jfloat t, jfloat r, jfloat bo)
{
	struct fpath *p = PATH(b);
	path_push(p, PO_MOVE, (float[]){l, t}, 2);
	path_push(p, PO_LINE, (float[]){r, t}, 2);
	path_push(p, PO_LINE, (float[]){r, bo}, 2);
	path_push(p, PO_LINE, (float[]){l, bo}, 2);
	path_push(p, PO_CLOSE, NULL, 0);
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1add_1round_1rect(JNIEnv *env, jclass c, jlong b, jfloat l, jfloat t, jfloat r, jfloat bo, jfloatArray radii)
{
	float rad[8] = {0};
	if (radii)
		(*env)->GetFloatArrayRegion(env, radii, 0, 8, rad);
	struct fpath *p = PATH(b);
	/* corners: top-left, top-right, bottom-right, bottom-left (rx, ry each) */
	path_arc(p, l, t, l + 2 * rad[0], t + 2 * rad[1], 180, 90, true);
	path_arc(p, r - 2 * rad[2], t, r, t + 2 * rad[3], 270, 90, false);
	path_arc(p, r - 2 * rad[4], bo - 2 * rad[5], r, bo, 0, 90, false);
	path_arc(p, l, bo - 2 * rad[7], l + 2 * rad[6], bo, 90, 90, false);
	path_push(p, PO_CLOSE, NULL, 0);
}

static cairo_t *scratch_cr(void)
{
	static cairo_t *cr;
	if (!cr) {
		cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_A8, 1, 1);
		cr = cairo_create(s);
		cairo_surface_destroy(s);
	}
	return cr;
}

JNIEXPORT void JNICALL Java_android_graphics_Path_native_1get_1bounds(JNIEnv *env, jclass c, jlong path, jobject rect)
{
	double x0 = 0, y0 = 0, x1 = 0, y1 = 0;
	if (path && PATH(path)->n) {
		cairo_t *cr = scratch_cr();
		path_replay(cr, PATH(path));
		cairo_path_extents(cr, &x0, &y0, &x1, &y1);
		cairo_new_path(cr);
	}
	_SET_FLOAT_FIELD(rect, "left", x0);
	_SET_FLOAT_FIELD(rect, "top", y0);
	_SET_FLOAT_FIELD(rect, "right", x1);
	_SET_FLOAT_FIELD(rect, "bottom", y1);
}

JNIEXPORT jlong JNICALL Java_android_graphics_Path_native_1transform(JNIEnv *env, jclass c, jlong path, jlong matrix)
{
	struct fpath *b = path_new();
	path_append(b, PATH(path), (float *)_PTR(matrix));
	path_unref(PATH(path));
	return _INTPTR(b);
}

JNIEXPORT jboolean JNICALL Java_android_graphics_Path_native_1contains(JNIEnv *env, jclass c, jlong path, jint rule, jboolean inverse, jfloat x, jfloat y)
{
	cairo_t *cr = scratch_cr();
	path_replay(cr, PATH(path));
	cairo_set_fill_rule(cr, rule == 1 ? CAIRO_FILL_RULE_EVEN_ODD : CAIRO_FILL_RULE_WINDING);
	bool in = cairo_in_fill(cr, x, y);
	cairo_new_path(cr);
	return in != !!inverse;
}

/* ---------------- Matrix: 3x3, Android order ---------------- */

#define MAT(ptr) ((float *)_PTR(ptr))

static void mat_identity(float *m)
{
	memset(m, 0, 9 * sizeof(float));
	m[0] = m[4] = m[8] = 1;
}

static void mat_mul(float *out, const float *a, const float *b)
{
	float r[9];
	for (int i = 0; i < 3; i++)
		for (int j = 0; j < 3; j++)
			r[i * 3 + j] = a[i * 3] * b[j] + a[i * 3 + 1] * b[3 + j] + a[i * 3 + 2] * b[6 + j];
	memcpy(out, r, sizeof(r));
}

static void mat_translate(float *m, float dx, float dy)
{
	mat_identity(m);
	m[2] = dx;
	m[5] = dy;
}

static void mat_scale(float *m, float sx, float sy, float px, float py)
{
	mat_identity(m);
	m[0] = sx;
	m[4] = sy;
	m[2] = px - sx * px;
	m[5] = py - sy * py;
}

static void mat_sincos(float *m, float s, float c, float px, float py)
{
	mat_identity(m);
	m[0] = c;
	m[1] = -s;
	m[3] = s;
	m[4] = c;
	m[2] = s * py + (1 - c) * px;
	m[5] = -s * px + (1 - c) * py;
}

static void mat_rotate(float *m, float deg, float px, float py)
{
	double r = deg * M_PI / 180;
	mat_sincos(m, sin(r), cos(r), px, py);
}

static void mat_skew(float *m, float kx, float ky, float px, float py)
{
	mat_identity(m);
	m[1] = kx;
	m[3] = ky;
	m[2] = -kx * py;
	m[5] = -ky * px;
}

JNIEXPORT jlong JNICALL Java_android_graphics_Matrix_native_1create(JNIEnv *env, jclass c, jlong src)
{
	float *m = malloc(9 * sizeof(float));
	if (src)
		memcpy(m, MAT(src), 9 * sizeof(float));
	else
		mat_identity(m);
	return _INTPTR(m);
}

JNIEXPORT void JNICALL Java_android_graphics_Matrix_finalizer(JNIEnv *env, jclass c, jlong m)
{
	free(MAT(m));
}

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1isIdentity(JNIEnv *env, jclass c, jlong m)
{
	float id[9];
	mat_identity(id);
	return !memcmp(id, MAT(m), sizeof(id));
}

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1rectStaysRect(JNIEnv *env, jclass c, jlong p)
{
	float *m = MAT(p);
	return (m[1] == 0 && m[3] == 0) || (m[0] == 0 && m[4] == 0);
}

JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1reset(JNIEnv *env, jclass c, jlong m) { mat_identity(MAT(m)); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1set(JNIEnv *env, jclass c, jlong m, jlong o)
{
	if (o)
		memcpy(MAT(m), MAT(o), 9 * sizeof(float));
	else
		mat_identity(MAT(m));
}
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setTranslate(JNIEnv *env, jclass c, jlong m, jfloat x, jfloat y) { mat_translate(MAT(m), x, y); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setScale__JFFFF(JNIEnv *env, jclass c, jlong m, jfloat sx, jfloat sy, jfloat px, jfloat py) { mat_scale(MAT(m), sx, sy, px, py); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setScale__JFF(JNIEnv *env, jclass c, jlong m, jfloat sx, jfloat sy) { mat_scale(MAT(m), sx, sy, 0, 0); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setRotate__JFFF(JNIEnv *env, jclass c, jlong m, jfloat d, jfloat px, jfloat py) { mat_rotate(MAT(m), d, px, py); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setRotate__JF(JNIEnv *env, jclass c, jlong m, jfloat d) { mat_rotate(MAT(m), d, 0, 0); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setSinCos__JFFFF(JNIEnv *env, jclass c, jlong m, jfloat s, jfloat co, jfloat px, jfloat py) { mat_sincos(MAT(m), s, co, px, py); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setSinCos__JFF(JNIEnv *env, jclass c, jlong m, jfloat s, jfloat co) { mat_sincos(MAT(m), s, co, 0, 0); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setSkew__JFFFF(JNIEnv *env, jclass c, jlong m, jfloat kx, jfloat ky, jfloat px, jfloat py) { mat_skew(MAT(m), kx, ky, px, py); }
JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setSkew__JFF(JNIEnv *env, jclass c, jlong m, jfloat kx, jfloat ky) { mat_skew(MAT(m), kx, ky, 0, 0); }

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1setConcat(JNIEnv *env, jclass c, jlong m, jlong a, jlong b)
{
	mat_mul(MAT(m), MAT(a), MAT(b));
	return true;
}

#define PREPOST(name, args, build)                                                                         \
	JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1pre##name(JNIEnv *env, jclass c, jlong p args) \
	{                                                                                                      \
		float t[9];                                                                                    \
		build;                                                                                         \
		mat_mul(MAT(p), MAT(p), t);                                                                    \
		return true;                                                                                   \
	}                                                                                                      \
	JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1post##name(JNIEnv *env, jclass c, jlong p args) \
	{                                                                                                      \
		float t[9];                                                                                    \
		build;                                                                                         \
		mat_mul(MAT(p), t, MAT(p));                                                                    \
		return true;                                                                                   \
	}
#define A(...) , __VA_ARGS__
PREPOST(Translate, A(jfloat x, jfloat y), mat_translate(t, x, y))
PREPOST(Scale__JFFFF, A(jfloat sx, jfloat sy, jfloat px, jfloat py), mat_scale(t, sx, sy, px, py))
PREPOST(Scale__JFF, A(jfloat sx, jfloat sy), mat_scale(t, sx, sy, 0, 0))
PREPOST(Rotate__JFFF, A(jfloat d, jfloat px, jfloat py), mat_rotate(t, d, px, py))
PREPOST(Rotate__JF, A(jfloat d), mat_rotate(t, d, 0, 0))
PREPOST(Skew__JFFFF, A(jfloat kx, jfloat ky, jfloat px, jfloat py), mat_skew(t, kx, ky, px, py))
PREPOST(Skew__JFF, A(jfloat kx, jfloat ky), mat_skew(t, kx, ky, 0, 0))
PREPOST(Concat, A(jlong o), memcpy(t, MAT(o), sizeof(t)))

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1setRectToRect(JNIEnv *env, jclass c, jlong p, jobject src, jobject dst, jint stf)
{
	float sl = _GET_FLOAT_FIELD(src, "left"), st = _GET_FLOAT_FIELD(src, "top");
	float sr = _GET_FLOAT_FIELD(src, "right"), sb = _GET_FLOAT_FIELD(src, "bottom");
	float dl = _GET_FLOAT_FIELD(dst, "left"), dt = _GET_FLOAT_FIELD(dst, "top");
	float dr = _GET_FLOAT_FIELD(dst, "right"), db = _GET_FLOAT_FIELD(dst, "bottom");
	float *m = MAT(p);
	if (sr <= sl || sb <= st) {
		mat_identity(m);
		return false;
	}
	float sx = (dr - dl) / (sr - sl), sy = (db - dt) / (sb - st);
	float tx = dl - sl * sx, ty = dt - st * sy;
	if (stf != 0) { /* FILL=0, START=1, CENTER=2, END=3: uniform scale */
		float s = sx < sy ? sx : sy;
		float dw = (sr - sl) * s, dh = (sb - st) * s;
		float ox = 0, oy = 0;
		if (stf == 2) {
			ox = ((dr - dl) - dw) / 2;
			oy = ((db - dt) - dh) / 2;
		} else if (stf == 3) {
			ox = (dr - dl) - dw;
			oy = (db - dt) - dh;
		}
		sx = sy = s;
		tx = dl - sl * s + ox;
		ty = dt - st * s + oy;
	}
	mat_identity(m);
	m[0] = sx;
	m[4] = sy;
	m[2] = tx;
	m[5] = ty;
	return true;
}

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1invert(JNIEnv *env, jclass c, jlong p, jlong out)
{
	float *m = MAT(p), r[9];
	double det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
	if (fabs(det) < 1e-12)
		return false;
	r[0] = (m[4] * m[8] - m[5] * m[7]) / det;
	r[1] = (m[2] * m[7] - m[1] * m[8]) / det;
	r[2] = (m[1] * m[5] - m[2] * m[4]) / det;
	r[3] = (m[5] * m[6] - m[3] * m[8]) / det;
	r[4] = (m[0] * m[8] - m[2] * m[6]) / det;
	r[5] = (m[2] * m[3] - m[0] * m[5]) / det;
	r[6] = (m[3] * m[7] - m[4] * m[6]) / det;
	r[7] = (m[1] * m[6] - m[0] * m[7]) / det;
	r[8] = (m[0] * m[4] - m[1] * m[3]) / det;
	if (out)
		memcpy(MAT(out), r, sizeof(r));
	return true;
}

JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1mapPoints(JNIEnv *env, jclass c, jlong p, jfloatArray dst, jint doff, jfloatArray src, jint soff, jint count, jboolean is_pts)
{
	float *m = MAT(p);
	float *s = (*env)->GetFloatArrayElements(env, src, NULL);
	float *d = (*env)->GetFloatArrayElements(env, dst, NULL);
	for (int i = 0; i < count; i++) {
		float x = s[soff + 2 * i], y = s[soff + 2 * i + 1];
		float tx = is_pts ? m[2] : 0, ty = is_pts ? m[5] : 0;
		float w = is_pts ? m[6] * x + m[7] * y + m[8] : 1;
		if (w == 0)
			w = 1;
		d[doff + 2 * i] = (m[0] * x + m[1] * y + tx) / w;
		d[doff + 2 * i + 1] = (m[3] * x + m[4] * y + ty) / w;
	}
	(*env)->ReleaseFloatArrayElements(env, src, s, JNI_ABORT);
	(*env)->ReleaseFloatArrayElements(env, dst, d, 0);
}

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1mapRect(JNIEnv *env, jclass c, jlong p, jobject dst, jobject src)
{
	float *m = MAT(p);
	float l = _GET_FLOAT_FIELD(src, "left"), t = _GET_FLOAT_FIELD(src, "top");
	float r = _GET_FLOAT_FIELD(src, "right"), b = _GET_FLOAT_FIELD(src, "bottom");
	float xs[4] = {l, r, r, l}, ys[4] = {t, t, b, b};
	float x0 = INFINITY, y0 = INFINITY, x1 = -INFINITY, y1 = -INFINITY;
	for (int i = 0; i < 4; i++) {
		float x = m[0] * xs[i] + m[1] * ys[i] + m[2], y = m[3] * xs[i] + m[4] * ys[i] + m[5];
		x0 = fminf(x0, x);
		y0 = fminf(y0, y);
		x1 = fmaxf(x1, x);
		y1 = fmaxf(y1, y);
	}
	_SET_FLOAT_FIELD(dst, "left", x0);
	_SET_FLOAT_FIELD(dst, "top", y0);
	_SET_FLOAT_FIELD(dst, "right", x1);
	_SET_FLOAT_FIELD(dst, "bottom", y1);
	return Java_android_graphics_Matrix_native_1rectStaysRect(env, c, p);
}

JNIEXPORT jfloat JNICALL Java_android_graphics_Matrix_native_1mapRadius(JNIEnv *env, jclass c, jlong p, jfloat r)
{
	float *m = MAT(p);
	return r * sqrtf(fabsf(m[0] * m[4] - m[1] * m[3]));
}

JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1getValues(JNIEnv *env, jclass c, jlong p, jfloatArray v)
{
	(*env)->SetFloatArrayRegion(env, v, 0, 9, MAT(p));
}

JNIEXPORT void JNICALL Java_android_graphics_Matrix_native_1setValues(JNIEnv *env, jclass c, jlong p, jfloatArray v)
{
	(*env)->GetFloatArrayRegion(env, v, 0, 9, MAT(p));
}

JNIEXPORT jboolean JNICALL Java_android_graphics_Matrix_native_1equals(JNIEnv *env, jclass c, jlong a, jlong b)
{
	return !memcmp(MAT(a), MAT(b), 9 * sizeof(float));
}

/* ---------------- GskCanvas ---------------- */

static void stroke_setup(cairo_t *cr, struct fpaint *p)
{
	cairo_set_line_width(cr, p->stroke_width > 0 ? p->stroke_width : 1);
	cairo_set_line_cap(cr, (cairo_line_cap_t)p->cap);
	cairo_set_line_join(cr, (cairo_line_join_t)p->join);
}

/* fill and/or stroke the current path */
static void paint_path(cairo_t *cr, struct fpaint *p)
{
	paint_source(cr, p);
	if (p->fill && p->stroke) {
		cairo_fill_preserve(cr);
		stroke_setup(cr, p);
		cairo_stroke(cr);
	} else if (p->stroke) {
		stroke_setup(cr, p);
		cairo_stroke(cr);
	} else {
		cairo_fill(cr);
	}
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawBitmap__JJIIIIJ(JNIEnv *env, jobject self, jlong cv, jlong tex, jint x, jint y, jint w, jint h, jlong paint)
{
	cairo_t *cr = CR(cv);
	cairo_save(cr);
	cairo_translate(cr, x, y);
	fp_draw(env, _PTR(tex), cr, w, h);
	cairo_restore(cr);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawBitmap__JJIIIIIIIIJ(JNIEnv *env, jobject self, jlong cv, jlong tex, jint x, jint y, jint w, jint h, jint sx, jint sy, jint sw, jint sh, jlong paint)
{
	struct fpaintable *p = _PTR(tex);
	if (!p || !p->surface || sw <= 0 || sh <= 0)
		return;
	cairo_t *cr = CR(cv);
	cairo_save(cr);
	cairo_rectangle(cr, x, y, w, h);
	cairo_clip(cr);
	cairo_translate(cr, x, y);
	cairo_scale(cr, (double)w / sw, (double)h / sh);
	cairo_set_source_surface(cr, p->surface, -sx, -sy);
	cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BILINEAR);
	cairo_paint(cr);
	cairo_restore(cr);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawRect(JNIEnv *env, jobject self, jlong cv, jfloat l, jfloat t, jfloat r, jfloat b, jlong paint)
{
	cairo_t *cr = CR(cv);
	cairo_new_path(cr);
	cairo_rectangle(cr, l, t, r - l, b - t);
	paint_path(cr, PAINT(paint));
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawPath(JNIEnv *env, jobject self, jlong cv, jlong path, jint rule, jlong paint)
{
	cairo_t *cr = CR(cv);
	path_replay(cr, PATH(path));
	cairo_set_fill_rule(cr, rule == 1 ? CAIRO_FILL_RULE_EVEN_ODD : CAIRO_FILL_RULE_WINDING);
	paint_path(cr, PAINT(paint));
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1translate(JNIEnv *env, jobject self, jlong cv, jfloat dx, jfloat dy) { cairo_translate(CR(cv), dx, dy); }
JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1rotate(JNIEnv *env, jobject self, jlong cv, jfloat deg) { cairo_rotate(CR(cv), deg * M_PI / 180); }
JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1scale(JNIEnv *env, jobject self, jlong cv, jfloat sx, jfloat sy)
{
	if (sx != 0 && sy != 0)
		cairo_scale(CR(cv), sx, sy);
}
JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1save(JNIEnv *env, jobject self, jlong cv) { cairo_save(CR(cv)); }
JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1restore(JNIEnv *env, jobject self, jlong cv) { cairo_restore(CR(cv)); }

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1concat(JNIEnv *env, jobject self, jlong cv, jlong mp)
{
	float *m = MAT(mp);
	cairo_matrix_t cm;
	cairo_matrix_init(&cm, m[0], m[3], m[1], m[4], m[2], m[5]);
	cairo_transform(CR(cv), &cm);
}

/* a clip lasts until GskCanvas pops it (native_pop), as with GtkSnapshot */
JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1clipRect(JNIEnv *env, jobject self, jlong cv, jfloat l, jfloat t, jfloat r, jfloat b)
{
	cairo_t *cr = CR(cv);
	cairo_save(cr);
	cairo_new_path(cr);
	cairo_rectangle(cr, l, t, r - l, b - t);
	cairo_clip(cr);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1clipPath(JNIEnv *env, jobject self, jlong cv, jlong path, jint rule)
{
	cairo_t *cr = CR(cv);
	cairo_save(cr);
	path_replay(cr, PATH(path));
	cairo_set_fill_rule(cr, rule == 1 ? CAIRO_FILL_RULE_EVEN_ODD : CAIRO_FILL_RULE_WINDING);
	cairo_clip(cr);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1pop(JNIEnv *env, jobject self, jlong cv, jint n)
{
	while (n-- > 0)
		cairo_restore(CR(cv));
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawLine(JNIEnv *env, jobject self, jlong cv, jfloat x0, jfloat y0, jfloat x1, jfloat y1, jlong paint)
{
	cairo_t *cr = CR(cv);
	struct fpaint *p = PAINT(paint);
	cairo_new_path(cr);
	cairo_move_to(cr, x0, y0);
	cairo_line_to(cr, x1, y1);
	paint_source(cr, p);
	stroke_setup(cr, p);
	cairo_stroke(cr);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawLines(JNIEnv *env, jobject self, jlong cv, jfloatArray pts, jint off, jint count, jlong paint)
{
	float *v = (*env)->GetFloatArrayElements(env, pts, NULL);
	for (int i = off; i + 3 < off + count; i += 4)
		Java_android_atl_GskCanvas_native_1drawLine(env, self, cv, v[i], v[i + 1], v[i + 2], v[i + 3], paint);
	(*env)->ReleaseFloatArrayElements(env, pts, v, JNI_ABORT);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawText(JNIEnv *env, jobject self, jlong cv, jstring text, jfloat x, jfloat y, jlong paint)
{
	cairo_t *cr = CR(cv);
	struct fpaint *p = PAINT(paint);
	const char *s = (*env)->GetStringUTFChars(env, text, NULL);
	PangoLayout *l = text_layout(cr, s, p->text_size, p->bold, p->italic);
	(*env)->ReleaseStringUTFChars(env, text, s);
	PangoRectangle log;
	pango_layout_get_pixel_extents(l, NULL, &log);
	y -= (double)pango_layout_get_baseline(l) / PANGO_SCALE;
	if (p->text_align == 1)
		x -= log.width / 2.0;
	else if (p->text_align == 2)
		x -= log.width;
	cairo_move_to(cr, x, y);
	paint_source(cr, p);
	pango_cairo_show_layout(cr, l);
	g_object_unref(l);
}

JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawRoundRect(JNIEnv *env, jobject self, jlong cv, jfloat l, jfloat t, jfloat r, jfloat b, jfloat rx, jfloat ry, jlong paint)
{
	cairo_t *cr = CR(cv);
	struct fpath *p = path_new();
	jlong pp = _INTPTR(p);
	float w = r - l, h = b - t;
	if (rx > w / 2)
		rx = w / 2;
	if (ry > h / 2)
		ry = h / 2;
	path_arc(p, l, t, l + 2 * rx, t + 2 * ry, 180, 90, true);
	path_arc(p, r - 2 * rx, t, r, t + 2 * ry, 270, 90, false);
	path_arc(p, r - 2 * rx, b - 2 * ry, r, b, 0, 90, false);
	path_arc(p, l, b - 2 * ry, l + 2 * rx, b, 90, 90, false);
	path_push(p, PO_CLOSE, NULL, 0);
	path_replay(cr, PATH(pp));
	paint_path(cr, PAINT(paint));
	path_unref(p);
}

/* RenderNode display lists are cairo recording surfaces */
JNIEXPORT void JNICALL Java_android_atl_GskCanvas_native_1drawRenderNode(JNIEnv *env, jobject self, jlong cv, jlong node)
{
	if (!node)
		return;
	cairo_t *cr = CR(cv);
	cairo_set_source_surface(cr, _PTR(node), 0, 0);
	cairo_paint(cr);
}

JNIEXPORT jlong JNICALL Java_android_view_RenderNode_nativeCreateSnapshot(JNIEnv *env, jobject self)
{
	return _INTPTR(fc_new(cairo_recording_surface_create(CAIRO_CONTENT_COLOR_ALPHA, NULL)));
}

JNIEXPORT jlong JNICALL Java_android_view_RenderNode_nativeCreateNode(JNIEnv *env, jobject self, jlong snap)
{
	struct fcanvas *c = _PTR(snap);
	cairo_surface_t *s = cairo_surface_reference(c->surface);
	fc_free(env, c);
	return _INTPTR(s);
}

JNIEXPORT jlong JNICALL Java_android_view_RenderNode_nativeCreateEmpyNode(JNIEnv *env, jobject self)
{
	return _INTPTR(cairo_recording_surface_create(CAIRO_CONTENT_COLOR_ALPHA, NULL));
}

static cairo_surface_t *wrap_node(cairo_surface_t *node, void (*fn)(cairo_t *, float *), float *args)
{
	cairo_surface_t *s = cairo_recording_surface_create(CAIRO_CONTENT_COLOR_ALPHA, NULL);
	cairo_t *cr = cairo_create(s);
	fn(cr, args);
	if (node) {
		cairo_set_source_surface(cr, node, 0, 0);
		cairo_paint(cr);
	}
	cairo_destroy(cr);
	return s;
}

static void do_transform(cairo_t *cr, float *a)
{
	/* translation, scale and rotation about a pivot, as RenderNode applies them */
	cairo_translate(cr, a[0] + a[5], a[1] + a[6]);
	cairo_rotate(cr, a[4] * M_PI / 180);
	cairo_scale(cr, a[2], a[3]);
	cairo_translate(cr, -a[5], -a[6]);
}

static void do_clip(cairo_t *cr, float *a)
{
	cairo_rectangle(cr, a[0], a[1], a[2] - a[0], a[3] - a[1]);
	cairo_clip(cr);
}

JNIEXPORT jlong JNICALL Java_android_view_RenderNode_nativeTransform(JNIEnv *env, jobject self, jlong node, jfloat tx, jfloat ty, jfloat sx, jfloat sy, jfloat rot, jfloat px, jfloat py)
{
	cairo_surface_t *s = wrap_node(_PTR(node), do_transform, (float[]){tx, ty, sx, sy, rot, px, py});
	if (node)
		cairo_surface_destroy(_PTR(node));
	return _INTPTR(s);
}

JNIEXPORT jlong JNICALL Java_android_view_RenderNode_nativeClip(JNIEnv *env, jobject self, jlong node, jfloat l, jfloat t, jfloat r, jfloat b)
{
	cairo_surface_t *s = wrap_node(_PTR(node), do_clip, (float[]){l, t, r, b});
	if (node)
		cairo_surface_destroy(_PTR(node));
	return _INTPTR(s);
}

JNIEXPORT void JNICALL Java_android_view_RenderNode_nativeUnref(JNIEnv *env, jobject self, jlong node)
{
	if (node)
		cairo_surface_destroy(_PTR(node));
}

/* ---------------- Bitmap ---------------- */
/* a "texture" is an FP_SURFACE paintable, a "snapshot" a canvas on its surface */

static struct fpaintable *tex_of_surface(cairo_surface_t *s)
{
	struct fpaintable *p = fp_new(FP_SURFACE);
	p->surface = s;
	return p;
}

/* A bitmap is either a texture (an image) or a snapshot (a canvas being
 * drawn on). Snapshots record into a cairo recording surface, as a
 * GtkSnapshot does, and become an image of the bitmap's size again in
 * create_texture. */
static struct fcanvas *recording(void)
{
	return fc_new(cairo_recording_surface_create(CAIRO_CONTENT_COLOR_ALPHA, NULL));
}

JNIEXPORT jlong JNICALL Java_android_graphics_Bitmap_native_1create_1snapshot(JNIEnv *env, jclass c, jlong tex)
{
	struct fcanvas *cv = recording();
	struct fpaintable *t = _PTR(tex);
	if (t) {
		if (t->surface) {
			cairo_set_source_surface(cv->cr, t->surface, 0, 0);
			cairo_paint(cv->cr);
		}
		fp_unref(t);
	}
	return _INTPTR(cv);
}

JNIEXPORT jlong JNICALL Java_android_graphics_Bitmap_native_1create_1texture(JNIEnv *env, jclass c, jlong snap, jint w, jint h, jint stride, jint fmt)
{
	cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w > 0 ? w : 1, h > 0 ? h : 1);
	if (snap) {
		struct fcanvas *cv = _PTR(snap);
		cairo_t *cr = cairo_create(s);
		cairo_set_source_surface(cr, cv->surface, 0, 0);
		cairo_paint(cr);
		cairo_destroy(cr);
		fc_free(env, cv);
	}
	return _INTPTR(tex_of_surface(s));
}

JNIEXPORT jint JNICALL Java_android_graphics_Bitmap_native_1get_1width(JNIEnv *env, jclass c, jlong tex)
{
	return fp_width(env, _PTR(tex));
}

JNIEXPORT jint JNICALL Java_android_graphics_Bitmap_native_1get_1height(JNIEnv *env, jclass c, jlong tex)
{
	return fp_height(env, _PTR(tex));
}

JNIEXPORT jlong JNICALL Java_android_graphics_Bitmap_native_1erase_1color(JNIEnv *env, jclass c, jint color, jint w, jint h)
{
	struct fcanvas *cv = recording();
	double r, g, b, a;
	color_split(color, &r, &g, &b, &a);
	cairo_set_operator(cv->cr, CAIRO_OPERATOR_SOURCE);
	cairo_set_source_rgba(cv->cr, r, g, b, a);
	cairo_rectangle(cv->cr, 0, 0, w, h);
	cairo_fill(cv->cr);
	cairo_set_operator(cv->cr, CAIRO_OPERATOR_OVER);
	return _INTPTR(cv);
}

JNIEXPORT void JNICALL Java_android_graphics_Bitmap_native_1recycle(JNIEnv *env, jclass c, jlong tex, jlong snap)
{
	if (tex)
		fp_unref(_PTR(tex));
	if (snap)
		fc_free(env, _PTR(snap));
}

JNIEXPORT jlong JNICALL Java_android_graphics_Bitmap_native_1ref_1texture(JNIEnv *env, jclass c, jlong tex)
{
	return _INTPTR(fp_ref(_PTR(tex)));
}

/* Android pixels are non-premultiplied ARGB ints; cairo's are premultiplied */
static uint32_t premul(uint32_t c)
{
	uint32_t a = c >> 24;
	if (a == 255)
		return c;
	uint32_t r = ((c >> 16) & 0xff) * a / 255, g = ((c >> 8) & 0xff) * a / 255, b = (c & 0xff) * a / 255;
	return (a << 24) | (r << 16) | (g << 8) | b;
}

static uint32_t unpremul(uint32_t c)
{
	uint32_t a = c >> 24;
	if (a == 255 || a == 0)
		return a ? c : 0;
	uint32_t r = ((c >> 16) & 0xff) * 255 / a, g = ((c >> 8) & 0xff) * 255 / a, b = (c & 0xff) * 255 / a;
	return (a << 24) | (r << 16) | (g << 8) | b;
}

JNIEXPORT void JNICALL Java_android_graphics_Bitmap_native_1get_1pixels(JNIEnv *env, jclass c, jlong tex, jintArray pixels, jint off, jint stride, jint x, jint y, jint w, jint h)
{
	struct fpaintable *t = _PTR(tex);
	if (!t || !t->surface)
		return;
	cairo_surface_flush(t->surface);
	unsigned char *data = cairo_image_surface_get_data(t->surface);
	int sstride = cairo_image_surface_get_stride(t->surface);
	jint *out = (*env)->GetIntArrayElements(env, pixels, NULL);
	for (int j = 0; j < h; j++)
		for (int i = 0; i < w; i++)
			out[off + j * stride + i] = unpremul(((uint32_t *)(data + (y + j) * sstride))[x + i]);
	(*env)->ReleaseIntArrayElements(env, pixels, out, 0);
}

JNIEXPORT void JNICALL Java_android_graphics_Bitmap_native_1set_1pixels(JNIEnv *env, jclass c, jlong snap, jintArray pixels, jint off, jint stride, jint x, jint y, jint w, jint h)
{
	struct fcanvas *cv = _PTR(snap);
	if (!cv || w <= 0 || h <= 0)
		return;
	cairo_surface_t *img = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w, h);
	unsigned char *data = cairo_image_surface_get_data(img);
	int istride = cairo_image_surface_get_stride(img);
	jint *in = (*env)->GetIntArrayElements(env, pixels, NULL);
	for (int j = 0; j < h; j++)
		for (int i = 0; i < w; i++)
			((uint32_t *)(data + j * istride))[i] = premul(in[off + j * stride + i]);
	(*env)->ReleaseIntArrayElements(env, pixels, in, JNI_ABORT);
	cairo_surface_mark_dirty(img);
	cairo_save(cv->cr);
	cairo_identity_matrix(cv->cr);
	cairo_set_operator(cv->cr, CAIRO_OPERATOR_SOURCE);
	cairo_set_source_surface(cv->cr, img, x, y);
	cairo_rectangle(cv->cr, x, y, w, h);
	cairo_fill(cv->cr);
	cairo_restore(cv->cr);
	cairo_surface_destroy(img);
}

/* ---------------- BitmapFactory: PNG via cairo ---------------- */

struct membuf {
	const unsigned char *p;
	size_t n, off;
};

static cairo_status_t read_mem(void *closure, unsigned char *data, unsigned int len)
{
	struct membuf *m = closure;
	if (m->off + len > m->n)
		return CAIRO_STATUS_READ_ERROR;
	memcpy(data, m->p + m->off, len);
	m->off += len;
	return CAIRO_STATUS_SUCCESS;
}

/* returns a texture or 0; Android's decoders return null for formats they cannot read */
jlong flare_decode_image(const unsigned char *p, size_t n)
{
	if (n < 8 || memcmp(p, "\x89PNG", 4)) {
		fprintf(stderr, "atl: image format not supported yet (%02x %02x %02x %02x)\n", n > 0 ? p[0] : 0, n > 1 ? p[1] : 0, n > 2 ? p[2] : 0, n > 3 ? p[3] : 0);
		return 0;
	}
	struct membuf m = {p, n, 0};
	cairo_surface_t *s = cairo_image_surface_create_from_png_stream(read_mem, &m);
	if (cairo_surface_status(s)) {
		cairo_surface_destroy(s);
		return 0;
	}
	return _INTPTR(tex_of_surface(s));
}

JNIEXPORT jlong JNICALL Java_android_graphics_BitmapFactory_nativeDecodeStream(JNIEnv *env, jclass c, jobject is, jbyteArray storage, jobject padding, jobject opts)
{
	/* read the whole stream through InputStream.read(byte[]) */
	jclass isc = (*env)->GetObjectClass(env, is);
	jmethodID rd = (*env)->GetMethodID(env, isc, "read", "([B)I");
	size_t cap = 65536, n = 0;
	unsigned char *buf = malloc(cap);
	jbyteArray chunk = (*env)->NewByteArray(env, 16384);
	for (;;) {
		jint got = (*env)->CallIntMethod(env, is, rd, chunk);
		if ((*env)->ExceptionCheck(env) || got <= 0)
			break;
		if (n + got > cap)
			buf = realloc(buf, cap = (n + got) * 2);
		(*env)->GetByteArrayRegion(env, chunk, 0, got, (jbyte *)buf + n);
		n += got;
	}
	if ((*env)->ExceptionCheck(env))
		(*env)->ExceptionClear(env);
	jlong t = flare_decode_image(buf, n);
	free(buf);
	return t;
}

/* ---------------- Drawable ---------------- */

JNIEXPORT jlong JNICALL Java_android_graphics_drawable_Drawable_native_1paintable_1from_1path(JNIEnv *env, jclass c, jstring path)
{
	const char *s = (*env)->GetStringUTFChars(env, path, NULL);
	cairo_surface_t *surf = cairo_image_surface_create_from_png(s);
	(*env)->ReleaseStringUTFChars(env, path, s);
	if (cairo_surface_status(surf)) {
		cairo_surface_destroy(surf);
		return 0;
	}
	return _INTPTR(tex_of_surface(surf));
}

JNIEXPORT jlong JNICALL Java_android_graphics_drawable_Drawable_native_1constructor(JNIEnv *env, jobject self)
{
	/* only Drawables that draw themselves in Java need a paintable calling back */
	jmethodID own = (*env)->GetMethodID(env, (*env)->GetObjectClass(env, self), "draw", "(Landroid/graphics/Canvas;)V");
	if (own == _CACHED_METHOD(Drawable__draw))
		return 0;
	struct fpaintable *p = fp_new(FP_JAVA);
	p->drawable = (*env)->NewWeakGlobalRef(env, self);
	return _INTPTR(p);
}

JNIEXPORT void JNICALL Java_android_graphics_drawable_Drawable_native_1invalidate(JNIEnv *env, jobject self, jlong p)
{
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_graphics_drawable_Drawable_native_1draw(JNIEnv *env, jobject self, jlong p, jlong cv, jint w, jint h)
{
	struct fpaintable *pt = _PTR(p);
	if (pt && pt->kind != FP_JAVA)
		fp_draw(env, pt, CR(cv), w, h);
}

JNIEXPORT void JNICALL Java_android_graphics_drawable_Drawable_native_1ref(JNIEnv *env, jobject self, jlong p) { fp_ref(_PTR(p)); }
JNIEXPORT void JNICALL Java_android_graphics_drawable_Drawable_native_1unref(JNIEnv *env, jobject self, jlong p) { fp_unref(_PTR(p)); }

/* a nine-patch: Android's compiled chunk gives the stretchable spans */
struct ninepatch_chunk {
	int8_t was_deserialized;
	uint8_t nx, ny, ncolors;
	uint32_t xoff, yoff;
	int32_t pad[4];
	uint32_t coloff;
} __attribute__((packed));

static jlong ninepatch(JNIEnv *env, cairo_surface_t *surface, const unsigned char *data, size_t n)
{
	if (!surface)
		return 0;
	struct fpaintable *p = fp_new(FP_NINEPATCH);
	p->surface = cairo_surface_reference(surface);
	int w = cairo_image_surface_get_width(surface), h = cairo_image_surface_get_height(surface);
	p->xdiv[0] = p->xdiv[1] = w / 2;
	p->ydiv[0] = p->ydiv[1] = h / 2;
	if (!data || n < sizeof(struct ninepatch_chunk))
		return _INTPTR(p);
	/* the chunk comes deserialized (offsets unset, host order) or as stored
	 * in a PNG (big-endian); as in ATL's NinePatchPaintable, check bounds */
	struct ninepatch_chunk c;
	memcpy(&c, data, sizeof(c));
	if (c.was_deserialized) {
		c.xoff = sizeof(c);
		c.yoff = c.xoff + c.nx * 4;
	}
	if (c.xoff > n || c.yoff > n || c.nx > (n - c.xoff) / 4 || c.ny > (n - c.yoff) / 4)
		return _INTPTR(p);
	int32_t xd[256], yd[256];
	int nx = c.nx < 256 ? c.nx : 256, ny = c.ny < 256 ? c.ny : 256;
	memcpy(xd, data + c.xoff, nx * 4);
	memcpy(yd, data + c.yoff, ny * 4);
	if (!c.was_deserialized) {
		for (int i = 0; i < nx; i++)
			xd[i] = (int32_t)ntohl((uint32_t)xd[i]);
		for (int i = 0; i < ny; i++)
			yd[i] = (int32_t)ntohl((uint32_t)yd[i]);
	}
	/* one stretchable span per axis: from the first to the last division */
	if (nx >= 2 && xd[0] >= 0 && xd[nx - 1] <= w && xd[0] <= xd[nx - 1]) {
		p->xdiv[0] = xd[0];
		p->xdiv[1] = xd[nx - 1];
	}
	if (ny >= 2 && yd[0] >= 0 && yd[ny - 1] <= h && yd[0] <= yd[ny - 1]) {
		p->ydiv[0] = yd[0];
		p->ydiv[1] = yd[ny - 1];
	}
	return _INTPTR(p);
}

JNIEXPORT jlong JNICALL Java_android_graphics_drawable_NinePatchDrawable_nativeCreate__Ljava_lang_String_2(JNIEnv *env, jobject self, jstring jpath)
{
	const char *path = (*env)->GetStringUTFChars(env, jpath, NULL);
	cairo_surface_t *s = cairo_image_surface_create_from_png(path);
	FILE *f = fopen(path, "rb");
	(*env)->ReleaseStringUTFChars(env, jpath, path);
	jlong r = 0;
	unsigned char *chunk = NULL;
	uint32_t size = 0;
	uint8_t buf[8];
	if (f && fread(buf, 1, 8, f) == 8 && !memcmp(buf, "\x89PNG", 4)) {
		while (fread(buf, 1, 8, f) == 8) {
			uint32_t len = ntohl(*(uint32_t *)buf);
			if (!memcmp(buf + 4, "npTc", 4)) {
				if (len >= sizeof(struct ninepatch_chunk) && len < 1024 * 1024) {
					chunk = malloc(len);
					if (fread(chunk, 1, len, f) == len)
						size = len;
				}
				break;
			}
			if (fseek(f, len + 4, SEEK_CUR))
				break;
		}
	}
	if (f)
		fclose(f);
	if (!cairo_surface_status(s) && size)
		r = ninepatch(env, s, chunk, size);
	free(chunk);
	cairo_surface_destroy(s);
	return r;
}

JNIEXPORT jlong JNICALL Java_android_graphics_drawable_NinePatchDrawable_nativeCreate___3BJ(JNIEnv *env, jobject self, jbyteArray chunk, jlong tex)
{
	jsize n = (*env)->GetArrayLength(env, chunk);
	jbyte *b = (*env)->GetByteArrayElements(env, chunk, NULL);
	struct fpaintable *t = _PTR(tex);
	jlong r = ninepatch(env, t ? t->surface : NULL, (unsigned char *)b, n);
	(*env)->ReleaseByteArrayElements(env, chunk, b, JNI_ABORT);
	return r;
}

JNIEXPORT void JNICALL Java_android_graphics_drawable_NinePatchDrawable_nativeSetTint(JNIEnv *env, jobject self, jlong p, jint tint)
{
	if (p)
		((struct fpaintable *)_PTR(p))->tint = tint;
}

/* DrawableContainer: a paintable forwarding to the selected child */
JNIEXPORT jlong JNICALL Java_android_graphics_drawable_DrawableContainer_native_1constructor(JNIEnv *env, jobject self)
{
	struct fpaintable *p = fp_new(FP_JAVA);
	p->drawable = (*env)->NewWeakGlobalRef(env, self);
	return _INTPTR(p);
}

JNIEXPORT void JNICALL Java_android_graphics_drawable_DrawableContainer_native_1selectChild(JNIEnv *env, jobject self, jlong p, jlong child)
{
	flare_request_frame();
}

/* ---------------- android.text.Layout on Pango ---------------- */

#define LAYOUT(p) ((PangoLayout *)_PTR(p))

JNIEXPORT jlong JNICALL Java_android_text_Layout_native_1constructor(JNIEnv *env, jclass c, jstring text, jlong paint, jint width)
{
	struct fpaint *p = PAINT(paint);
	const char *s = text ? (*env)->GetStringUTFChars(env, text, NULL) : NULL;
	PangoLayout *l = text_layout(NULL, s, p->text_size, p->bold, p->italic);
	if (s)
		(*env)->ReleaseStringUTFChars(env, text, s);
	pango_layout_set_width(l, width < 0 ? -1 : width * PANGO_SCALE);
	pango_layout_set_wrap(l, PANGO_WRAP_WORD_CHAR);
	return _INTPTR(l);
}

JNIEXPORT void JNICALL Java_android_text_Layout_native_1set_1width(JNIEnv *env, jobject o, jlong l, jint w) { pango_layout_set_width(LAYOUT(l), w * PANGO_SCALE); }
JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1width(JNIEnv *env, jobject o, jlong l) { return pango_layout_get_width(LAYOUT(l)) / PANGO_SCALE; }

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1height(JNIEnv *env, jobject o, jlong l)
{
	PangoRectangle r;
	pango_layout_get_extents(LAYOUT(l), NULL, &r);
	return r.height / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1count(JNIEnv *env, jobject o, jlong l) { return pango_layout_get_line_count(LAYOUT(l)); }

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1start(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoLayoutLine *pl = pango_layout_get_line_readonly(LAYOUT(l), line);
	return pl ? g_utf8_strlen(pango_layout_get_text(LAYOUT(l)), pango_layout_line_get_start_index(pl)) : 0;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1end(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoLayoutLine *pl = pango_layout_get_line_readonly(LAYOUT(l), line);
	if (!pl)
		return 0;
	return g_utf8_strlen(pango_layout_get_text(LAYOUT(l)), pango_layout_line_get_start_index(pl) + pango_layout_line_get_length(pl));
}

static void line_extents(PangoLayout *l, int line, PangoRectangle *r, int *baseline)
{
	PangoLayoutIter *it = pango_layout_get_iter(l);
	for (int i = 0; i < line && pango_layout_iter_next_line(it); i++)
		;
	pango_layout_iter_get_line_extents(it, NULL, r);
	if (baseline)
		*baseline = pango_layout_iter_get_baseline(it);
	pango_layout_iter_free(it);
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1top(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	line_extents(LAYOUT(l), line, &r, NULL);
	return r.y / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1bottom(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	line_extents(LAYOUT(l), line, &r, NULL);
	return (r.y + r.height) / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1left(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	line_extents(LAYOUT(l), line, &r, NULL);
	return r.x / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1right(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	line_extents(LAYOUT(l), line, &r, NULL);
	return (r.x + r.width) / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1width(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	line_extents(LAYOUT(l), line, &r, NULL);
	return r.width / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1baseline(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	int b;
	line_extents(LAYOUT(l), line, &r, &b);
	return b / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1ascent(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	int b;
	line_extents(LAYOUT(l), line, &r, &b);
	return (r.y - b) / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1descent(JNIEnv *env, jobject o, jlong l, jint line)
{
	PangoRectangle r;
	int b;
	line_extents(LAYOUT(l), line, &r, &b);
	return (r.y + r.height - b) / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1for_1vertical(JNIEnv *env, jobject o, jlong l, jint y)
{
	int idx, trailing, line;
	pango_layout_xy_to_index(LAYOUT(l), 0, y * PANGO_SCALE, &idx, &trailing);
	pango_layout_index_to_line_x(LAYOUT(l), idx, FALSE, &line, NULL);
	return line;
}

JNIEXPORT void JNICALL Java_android_text_Layout_native_1set_1ellipsize(JNIEnv *env, jobject o, jlong l, jint mode, jfloat w)
{
	pango_layout_set_ellipsize(LAYOUT(l), (PangoEllipsizeMode)mode);
	pango_layout_set_width(LAYOUT(l), w * PANGO_SCALE);
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1ellipsis_1count(JNIEnv *env, jobject o, jlong l, jint line) { return pango_layout_is_ellipsized(LAYOUT(l)); }

JNIEXPORT void JNICALL Java_android_text_Layout_native_1draw(JNIEnv *env, jobject o, jlong l, jlong cv, jlong paint)
{
	cairo_t *cr = CR(cv);
	cairo_move_to(cr, 0, 0);
	paint_source(cr, PAINT(paint));
	pango_cairo_update_layout(cr, LAYOUT(l));
	pango_cairo_show_layout(cr, LAYOUT(l));
}

JNIEXPORT void JNICALL Java_android_text_Layout_native_1draw_1custom_1canvas(JNIEnv *env, jobject o, jlong l, jobject canvas, jobject paint)
{
	PangoLayout *pl = LAYOUT(l);
	const char *text = pango_layout_get_text(pl);
	PangoLayoutIter *it = pango_layout_get_iter(pl);
	do {
		PangoLayoutLine *line = pango_layout_iter_get_line_readonly(it);
		char *seg = g_strndup(text + line->start_index, line->length);
		jstring js = (*env)->NewStringUTF(env, seg);
		g_free(seg);
		jint end = (*env)->GetStringLength(env, js);
		J__Canvas__drawText(env, canvas, js, 0, end, 0.f, (jfloat)pango_layout_iter_get_baseline(it) / PANGO_SCALE, paint);
		(*env)->DeleteLocalRef(env, js);
	} while (pango_layout_iter_next_line(it));
	pango_layout_iter_free(it);
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1line_1for_1offset(JNIEnv *env, jclass c, jlong l, jint off)
{
	int line;
	const char *t = pango_layout_get_text(LAYOUT(l));
	pango_layout_index_to_line_x(LAYOUT(l), g_utf8_offset_to_pointer(t, off) - t, FALSE, &line, NULL);
	return line;
}

JNIEXPORT jfloat JNICALL Java_android_text_Layout_native_1get_1primary_1horizontal(JNIEnv *env, jclass c, jlong l, jint off)
{
	PangoRectangle r;
	const char *t = pango_layout_get_text(LAYOUT(l));
	pango_layout_get_cursor_pos(LAYOUT(l), g_utf8_offset_to_pointer(t, off) - t, &r, NULL);
	return (jfloat)r.x / PANGO_SCALE;
}

JNIEXPORT jfloat JNICALL Java_android_text_Layout_native_1get_1secondary_1horizontal(JNIEnv *env, jclass c, jlong l, jint off)
{
	PangoRectangle r;
	const char *t = pango_layout_get_text(LAYOUT(l));
	pango_layout_get_cursor_pos(LAYOUT(l), g_utf8_offset_to_pointer(t, off) - t, NULL, &r);
	return (jfloat)r.x / PANGO_SCALE;
}

JNIEXPORT jint JNICALL Java_android_text_Layout_native_1get_1offset_1for_1horizontal(JNIEnv *env, jclass c, jlong l, jint line, jfloat x)
{
	PangoLayoutLine *pl = pango_layout_get_line_readonly(LAYOUT(l), line);
	int idx = 0, trailing;
	if (pl)
		pango_layout_line_x_to_index(pl, x * PANGO_SCALE, &idx, &trailing);
	return g_utf8_strlen(pango_layout_get_text(LAYOUT(l)), idx);
}

JNIEXPORT jfloat JNICALL Java_android_text_Layout_native_1get_1desired_1width(JNIEnv *env, jclass c, jlong l)
{
	int w;
	pango_layout_get_size(LAYOUT(l), &w, NULL);
	return (jfloat)w / PANGO_SCALE;
}

JNIEXPORT void JNICALL Java_android_text_Layout_native_1free(JNIEnv *env, jclass c, jlong l) { g_object_unref(LAYOUT(l)); }
