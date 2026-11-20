/* android.view.View/ViewGroup and the framework widgets ATL backs natively
 * (TextView, Button, EditText, ImageView, ImageButton, ScrollView), as
 * Flare view nodes.
 * SPDX-License-Identifier: GPL-3.0-or-later (derived from ATL) */
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "flare.h"

#define GONE      8
#define INVISIBLE 4

static jfieldID widget_field(JNIEnv *env)
{
	static jfieldID f;
	if (!f)
		f = (*env)->GetFieldID(env, _CACHED_CLASS(View), "widget", "J");
	return f;
}

struct fw *fw_of(JNIEnv *env, jobject view)
{
	return view ? _PTR((*env)->GetLongField(env, view, widget_field(env))) : NULL;
}

struct fw *fw_new(JNIEnv *env, jobject view, int kind)
{
	struct fw *w = calloc(1, sizeof(*w));
	w->view = (*env)->NewWeakGlobalRef(env, view);
	w->kind = kind;
	w->visible = true;
	w->alpha = 1;
	w->color = 0xff202020;
	w->text_size = 15 * flare_density;
	w->align = -1;
	return w;
}

static void fw_detach(struct fw *child)
{
	struct fw *p = child->parent;
	if (!p)
		return;
	for (int i = 0; i < p->nkids; i++)
		if (p->kids[i] == child) {
			memmove(&p->kids[i], &p->kids[i + 1], (p->nkids - i - 1) * sizeof(*p->kids));
			p->nkids--;
			break;
		}
	child->parent = NULL;
}

static void fw_attach(struct fw *p, struct fw *child, int index)
{
	fw_detach(child);
	if (p->nkids == p->cap) {
		p->cap = p->cap ? p->cap * 2 : 4;
		p->kids = realloc(p->kids, p->cap * sizeof(*p->kids));
	}
	if (index < 0 || index > p->nkids)
		index = p->nkids;
	memmove(&p->kids[index + 1], &p->kids[index], (p->nkids - index) * sizeof(*p->kids));
	p->kids[index] = child;
	p->nkids++;
	child->parent = p;
}

static char *dup_jstring(JNIEnv *env, jstring s)
{
	if (!s)
		return NULL;
	const char *c = (*env)->GetStringUTFChars(env, s, NULL);
	char *r = strdup(c);
	(*env)->ReleaseStringUTFChars(env, s, c);
	return r;
}

/* ---------------- View ---------------- */

JNIEXPORT jlong JNICALL Java_android_view_View_native_1constructor(JNIEnv *env, jobject self, jobject context, jobject attrs)
{
	bool group = (*env)->IsInstanceOf(env, self, _CACHED_CLASS(ViewGroup));
	return _INTPTR(fw_new(env, self, group ? FW_GROUP : FW_VIEW));
}

JNIEXPORT void JNICALL Java_android_view_View_native_1destructor(JNIEnv *env, jobject self, jlong p)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	fw_detach(w);
	for (int i = 0; i < w->nkids; i++)
		w->kids[i]->parent = NULL;
	if (focused_edit == w)
		focused_edit = NULL;
	(*env)->DeleteWeakGlobalRef(env, w->view);
	for (int i = 0; i < w->nwatchers; i++)
		(*env)->DeleteGlobalRef(env, w->watchers[i]);
	if (w->editor_action)
		(*env)->DeleteGlobalRef(env, w->editor_action);
	fp_unref(w->bg);
	fp_unref(w->image);
	for (int i = 0; i < 4; i++)
		fp_unref(w->compound[i]);
	free(w->kids);
	free(w->text);
	free(w->hint);
	free(w);
}

JNIEXPORT void JNICALL Java_android_view_View_native_1setLayoutParams(JNIEnv *env, jobject self, jlong p, jint width, jint height, jint gravity, jfloat weight, jint l, jint t, jint r, jint b)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	w->lp_w = width;
	w->lp_h = height;
}

JNIEXPORT void JNICALL Java_android_view_View_native_1setPadding(JNIEnv *env, jobject self, jlong p, jint l, jint t, jint r, jint b)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	w->pad[0] = l;
	w->pad[1] = t;
	w->pad[2] = r;
	w->pad[3] = b;
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_view_View_native_1setVisibility(JNIEnv *env, jobject self, jlong p, jint vis, jfloat alpha, jboolean sensitive)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	bool v = vis == 0;
	if (w->visible != v || w->alpha != alpha) {
		w->visible = v;
		w->alpha = alpha;
		flare_request_frame();
	}
}

JNIEXPORT void JNICALL Java_android_view_View_nativeInvalidate(JNIEnv *env, jclass c, jlong p)
{
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_view_View_native_1requestLayout(JNIEnv *env, jobject self, jlong p)
{
	flare_request_layout();
}

JNIEXPORT void JNICALL Java_android_view_View_native_1queueAllocate(JNIEnv *env, jobject self, jlong p)
{
	flare_request_layout();
}

/* the natural size of a node's content (without padding) */
static void content_size(JNIEnv *env, struct fw *w, int for_width, int *cw, int *ch)
{
	*cw = *ch = 0;
	switch (w->kind) {
		case FW_TEXT:
		case FW_BUTTON:
		case FW_EDIT:
			fw_text_size(w, for_width, cw, ch);
			if (w->kind == FW_BUTTON && !w->bg) {
				*cw += 32 * flare_density;
				*ch += 20 * flare_density;
			}
			if (w->kind == FW_EDIT)
				*ch += 12 * flare_density;
			for (int i = 0; i < 4; i++)
				if (w->compound[i]) {
					if (i % 2 == 0) {
						*cw += fp_width(env, w->compound[i]) + 4;
						int h = fp_height(env, w->compound[i]);
						*ch = *ch > h ? *ch : h;
					} else {
						*ch += fp_height(env, w->compound[i]) + 4;
					}
				}
			break;
		case FW_IMAGE:
			*cw = fp_width(env, w->image);
			*ch = fp_height(env, w->image);
			break;
		default:
			if (w->bg) {
				*cw = fp_width(env, w->bg);
				*ch = fp_height(env, w->bg);
			}
			break;
	}
}

static int resolve(int spec, int natural)
{
	int mode = spec & MEASURE_SPEC_MASK, size = spec & ~MEASURE_SPEC_MASK;
	if (mode == MEASURE_SPEC_EXACTLY)
		return size;
	if (mode == MEASURE_SPEC_AT_MOST && natural > size)
		return size;
	return natural;
}

JNIEXPORT void JNICALL Java_android_view_View_native_1measure(JNIEnv *env, jobject self, jlong p, jint wspec, jint hspec)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	int padx = w->pad[0] + w->pad[2], pady = w->pad[1] + w->pad[3];
	int avail = -1;
	if ((wspec & MEASURE_SPEC_MASK) != 0)
		avail = (wspec & ~MEASURE_SPEC_MASK) - padx;
	int cw, ch;
	content_size(env, w, avail, &cw, &ch);
	int width = resolve(wspec, cw + padx);
	if (width - padx < cw && w->kind != FW_IMAGE)
		content_size(env, w, width - padx, &cw, &ch);
	if (w->kind == FW_IMAGE && cw > 0 && width - padx < cw)
		ch = ch * (width - padx) / cw; /* keep the aspect ratio when narrowed */
	int height = resolve(hspec, ch + pady);
	J__View__setMeasuredDimension(env, self, width, height);
}

JNIEXPORT void JNICALL Java_android_view_View_native_1layout(JNIEnv *env, jobject self, jlong p, jint l, jint t, jint r, jint b)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	w->x = l;
	w->y = t;
	w->w = r - l;
	w->h = b - t;
	J__View__layoutInternal(env, self, r - l, b - t);
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_view_View_native_1setBackgroundColor(JNIEnv *env, jobject self, jlong p, jint color)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	fp_unref(w->bg);
	w->bg = NULL;
	w->has_bg_color = true;
	w->bg_color = color;
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_view_View_native_1setBackgroundDrawable(JNIEnv *env, jobject self, jlong p, jlong paintable)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	fp_unref(w->bg);
	w->bg = fp_ref(_PTR(paintable));
	w->has_bg_color = false;
	flare_request_frame();
}

static void classes(JNIEnv *env, struct fw *w, jstring name, bool on)
{
	const char *c = (*env)->GetStringUTFChars(env, name, NULL);
	if (!strcmp(c, "ATL-font-bold"))
		w->bold = on;
	else if (!strcmp(c, "ATL-font-italic"))
		w->italic = on;
	else if (!strcmp(c, "ATL-text-uppercase"))
		w->upper = on;
	else if (!strcmp(c, "ATL-no-border"))
		w->no_border = on;
	else if (on && !strcmp(c, "ATL-text-align-left"))
		w->align = 0;
	else if (on && !strcmp(c, "ATL-text-align-center"))
		w->align = 1;
	else if (on && !strcmp(c, "ATL-text-align-right"))
		w->align = 2;
	else if (!on && !strncmp(c, "ATL-text-align-", 15))
		w->align = -1;
	(*env)->ReleaseStringUTFChars(env, name, c);
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_view_View_native_1addClass(JNIEnv *env, jobject self, jlong p, jstring name) { classes(env, _PTR(p), name, true); }
JNIEXPORT void JNICALL Java_android_view_View_native_1removeClass(JNIEnv *env, jobject self, jlong p, jstring name) { classes(env, _PTR(p), name, false); }

static void classes_array(JNIEnv *env, struct fw *w, jobjectArray a, bool on)
{
	int n = (*env)->GetArrayLength(env, a);
	for (int i = 0; i < n; i++) {
		jstring s = (*env)->GetObjectArrayElement(env, a, i);
		classes(env, w, s, on);
		(*env)->DeleteLocalRef(env, s);
	}
}

JNIEXPORT void JNICALL Java_android_view_View_native_1addClasses(JNIEnv *env, jobject self, jlong p, jobjectArray a) { classes_array(env, _PTR(p), a, true); }
JNIEXPORT void JNICALL Java_android_view_View_native_1removeClasses(JNIEnv *env, jobject self, jlong p, jobjectArray a) { classes_array(env, _PTR(p), a, false); }

static cairo_t *cr_of(jlong snap)
{
	return ((struct fcanvas *)_PTR(snap))->cr;
}

JNIEXPORT void JNICALL Java_android_view_View_native_1drawBackground(JNIEnv *env, jobject self, jlong p, jlong snap)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	cairo_t *cr = cr_of(snap);
	int width = w->w, height = w->h;
	if (w->has_bg_color) {
		double r, g, b, a;
		color_split(w->bg_color, &r, &g, &b, &a);
		cairo_set_source_rgba(cr, r, g, b, a);
		cairo_rectangle(cr, 0, 0, width, height);
		cairo_fill(cr);
	} else if (w->bg) {
		fp_draw(env, w->bg, cr, width, height);
	}
}

/* ---------------- text ---------------- */

static char *display_text(struct fw *w, bool *is_hint)
{
	const char *t = w->text;
	*is_hint = false;
	if ((!t || !*t) && w->hint && *w->hint) {
		t = w->hint;
		*is_hint = true;
	}
	char *s = strdup(t ? t : "");
	if (w->upper)
		for (char *c = s; *c; c++)
			*c = toupper((unsigned char)*c);
	return s;
}

static PangoLayout *fw_layout(struct fw *w, cairo_t *cr, int width, bool *is_hint)
{
	char *s = display_text(w, is_hint);
	PangoLayout *l = text_layout(cr, NULL, w->text_size, w->bold || w->kind == FW_BUTTON, w->italic);
	if (w->markup && !*is_hint)
		pango_layout_set_markup(l, s, -1);
	else
		pango_layout_set_text(l, s, -1);
	free(s);
	if (width > 0 && w->kind != FW_EDIT) {
		pango_layout_set_width(l, width * PANGO_SCALE);
		pango_layout_set_wrap(l, PANGO_WRAP_WORD_CHAR);
	} else if (w->kind == FW_EDIT) {
		pango_layout_set_single_paragraph_mode(l, TRUE);
	}
	return l;
}

void fw_text_size(struct fw *w, int for_width, int *out_w, int *out_h)
{
	bool hint;
	PangoLayout *l = fw_layout(w, NULL, for_width, &hint);
	pango_layout_get_pixel_size(l, out_w, out_h);
	g_object_unref(l);
	if (w->kind == FW_EDIT && *out_w < 120 * flare_density)
		*out_w = 120 * flare_density;
}

static int gravity_of(JNIEnv *env, jobject view)
{
	jclass c = (*env)->GetObjectClass(env, view);
	jmethodID m = (*env)->GetMethodID(env, c, "getGravity", "()I");
	(*env)->DeleteLocalRef(env, c);
	int g = (*env)->CallIntMethod(env, view, m);
	if ((*env)->ExceptionCheck(env)) {
		(*env)->ExceptionClear(env);
		return 0x11;
	}
	return g;
}

static void rounded(cairo_t *cr, double x, double y, double w, double h, double r)
{
	cairo_new_sub_path(cr);
	cairo_arc(cr, x + w - r, y + r, r, -G_PI / 2, 0);
	cairo_arc(cr, x + w - r, y + h - r, r, 0, G_PI / 2);
	cairo_arc(cr, x + r, y + h - r, r, G_PI / 2, G_PI);
	cairo_arc(cr, x + r, y + r, r, G_PI, 3 * G_PI / 2);
	cairo_close_path(cr);
}

static void draw_text_widget(JNIEnv *env, jobject self, struct fw *w, cairo_t *cr)
{
	int width = w->w, height = w->h;
	double d = flare_density;
	if (w->kind == FW_BUTTON && !w->bg && !w->has_bg_color) {
		/* a Flare-style button: a soft rounded slab, darker while pressed */
		rounded(cr, 2 * d, 2 * d, width - 4 * d, height - 4 * d, 6 * d);
		if (w->pressed)
			cairo_set_source_rgb(cr, 0.80, 0.84, 0.90);
		else
			cairo_set_source_rgb(cr, 0.93, 0.94, 0.96);
		cairo_fill_preserve(cr);
		cairo_set_source_rgba(cr, 0, 0, 0, 0.18);
		cairo_set_line_width(cr, 1);
		cairo_stroke(cr);
	}
	if (w->kind == FW_EDIT) {
		bool focused = focused_edit == w;
		cairo_set_source_rgba(cr, focused ? 0.0 : 0.5, focused ? 0.45 : 0.5, focused ? 0.85 : 0.5, 1);
		cairo_rectangle(cr, w->pad[0], height - w->pad[3] - (focused ? 2 : 1) * d, width - w->pad[0] - w->pad[2], (focused ? 2 : 1) * d);
		cairo_fill(cr);
	}
	int iw = width - w->pad[0] - w->pad[2], ih = height - w->pad[1] - w->pad[3];
	bool hint;
	PangoLayout *l = fw_layout(w, cr, iw, &hint);
	int tw, th;
	pango_layout_get_pixel_size(l, &tw, &th);
	int g = w->kind == FW_EDIT ? 0x13 : gravity_of(env, self);
	int hg = g & 0x07, vg = g & 0x70;
	int align = w->align;
	if (align < 0)
		align = hg == 0x01 ? 1 : hg == 0x05 ? 2 : 0;
	pango_layout_set_alignment(l, align == 1 ? PANGO_ALIGN_CENTER : align == 2 ? PANGO_ALIGN_RIGHT : PANGO_ALIGN_LEFT);
	double x = w->pad[0], y = w->pad[1];
	if (align == 1 && tw < iw)
		x += (iw - tw) / 2.0;
	else if (align == 2 && tw < iw)
		x += iw - tw;
	if (vg == 0x10 || w->kind == FW_EDIT)
		y += (ih - th) / 2.0;
	else if (vg == 0x50)
		y += ih - th;
	/* a centred/right layout already places lines within its width */
	if (align && pango_layout_get_width(l) > 0)
		x = w->pad[0];
	double r, gg, b, a;
	color_split(w->color, &r, &gg, &b, &a);
	if (hint)
		a *= 0.45;
	cairo_set_source_rgba(cr, r, gg, b, a);
	cairo_move_to(cr, x, y);
	pango_cairo_show_layout(cr, l);
	if (w->kind == FW_EDIT && focused_edit == w) {
		/* a caret after the text */
		int cx = hint ? 0 : tw;
		cairo_rectangle(cr, x + cx + 1, y, 1.5 * d, th);
		cairo_set_source_rgb(cr, 0, 0.45, 0.85);
		cairo_fill(cr);
	}
	g_object_unref(l);
}

JNIEXPORT void JNICALL Java_android_view_View_native_1drawContent(JNIEnv *env, jobject self, jlong p, jlong snap)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	cairo_t *cr = cr_of(snap);
	switch (w->kind) {
		case FW_TEXT:
		case FW_BUTTON:
		case FW_EDIT:
			draw_text_widget(env, self, w, cr);
			break;
		case FW_IMAGE: {
			int width = w->w, height = w->h;
			int iw = width - w->pad[0] - w->pad[2], ih = height - w->pad[1] - w->pad[3];
			int nw = fp_width(env, w->image), nh = fp_height(env, w->image);
			if (nw <= 0 || nh <= 0) {
				nw = iw;
				nh = ih;
			}
			/* FIT_CENTER unless FIT_XY/CENTER_CROP/CENTER asked for */
			double sx = (double)iw / nw, sy = (double)ih / nh, s = sx < sy ? sx : sy;
			double dw = nw * s, dh = nh * s;
			if (w->scale_type == 1 /* FIT_XY */) {
				dw = iw;
				dh = ih;
			} else if (w->scale_type == 6 /* CENTER_CROP */) {
				s = sx > sy ? sx : sy;
				dw = nw * s;
				dh = nh * s;
			} else if (w->scale_type == 5 /* CENTER */) {
				dw = nw;
				dh = nh;
			}
			cairo_save(cr);
			cairo_rectangle(cr, w->pad[0], w->pad[1], iw, ih);
			cairo_clip(cr);
			cairo_translate(cr, w->pad[0] + (iw - dw) / 2, w->pad[1] + (ih - dh) / 2);
			fp_draw(env, w->image, cr, dw, dh);
			cairo_restore(cr);
			break;
		}
		default:
			break;
	}
}

/* ---------------- ViewGroup ---------------- */

JNIEXPORT void JNICALL Java_android_view_ViewGroup_native_1addView(JNIEnv *env, jobject self, jlong p, jlong child, jint index, jobject lp)
{
	if (p && child)
		fw_attach(_PTR(p), _PTR(child), index);
	flare_request_layout();
}

JNIEXPORT void JNICALL Java_android_view_ViewGroup_native_1removeView(JNIEnv *env, jobject self, jlong p, jlong child)
{
	if (child)
		fw_detach(_PTR(child));
	flare_request_layout();
}

static jobject canvas_object(JNIEnv *env, jlong snap)
{
	struct fcanvas *c = _PTR(snap);
	if (!c->jcanvas) {
		jobject o = J_new__GskCanvas(env, snap);
		c->jcanvas = (*env)->NewGlobalRef(env, o);
		(*env)->DeleteLocalRef(env, o);
	}
	return c->jcanvas;
}

static float view_float(JNIEnv *env, jobject v, const char *name)
{
	static jfieldID tx, ty;
	jfieldID *f = name[11] == 'X' ? &tx : &ty;
	if (!*f)
		*f = (*env)->GetFieldID(env, _CACHED_CLASS(View), name, "F");
	return (*env)->GetFloatField(env, v, *f);
}

static void draw_child(JNIEnv *env, jobject parent, struct fw *c, jlong snap)
{
	if (!c->visible || c->alpha <= 0)
		return;
	jobject v = (*env)->NewLocalRef(env, c->view);
	if (!v)
		return;
	cairo_t *cr = cr_of(snap);
	int w = c->w, h = c->h;
	int sx = parent ? J__View__getScrollX(env, parent) : 0, sy = parent ? J__View__getScrollY(env, parent) : 0;
	cairo_save(cr);
	cairo_translate(cr, c->x - sx + view_float(env, v, "translationX"), c->y - sy + view_float(env, v, "translationY"));
	cairo_rectangle(cr, 0, 0, w, h);
	cairo_clip(cr);
	if (c->alpha < 1)
		cairo_push_group(cr);
	int csx = J__View__getScrollX(env, v), csy = J__View__getScrollY(env, v);
	J__View__draw(env, v, canvas_object(env, snap));
	(void)csx;
	(void)csy;
	if ((*env)->ExceptionCheck(env))
		{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
	if (c->alpha < 1) {
		cairo_pop_group_to_source(cr);
		cairo_paint_with_alpha(cr, c->alpha);
	}
	cairo_restore(cr);
	(*env)->DeleteLocalRef(env, v);
}

JNIEXPORT void JNICALL Java_android_view_ViewGroup_native_1drawChildren(JNIEnv *env, jobject self, jlong p, jlong snap)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	/* children can be added or removed while drawing: walk a copy */
	int n = w->nkids;
	struct fw **kids = malloc(n * sizeof(*kids) + 1);
	memcpy(kids, w->kids, n * sizeof(*kids));
	for (int i = 0; i < n; i++)
		draw_child(env, self, kids[i], snap);
	free(kids);
}

JNIEXPORT void JNICALL Java_android_view_ViewGroup_native_1drawChild(JNIEnv *env, jobject self, jlong p, jlong child, jlong snap)
{
	if (child)
		draw_child(env, self, _PTR(child), snap);
}

JNIEXPORT void JNICALL Java_android_view_ViewGroup_native_1offsetRect(JNIEnv *env, jobject self, jlong from, jlong to, jobject rect)
{
	int dx = 0, dy = 0;
	for (struct fw *w = _PTR(from); w && w != _PTR(to); w = w->parent) {
		dx += w->x;
		dy += w->y;
	}
	_SET_INT_FIELD(rect, "left", _GET_INT_FIELD(rect, "left") + dx);
	_SET_INT_FIELD(rect, "right", _GET_INT_FIELD(rect, "right") + dx);
	_SET_INT_FIELD(rect, "top", _GET_INT_FIELD(rect, "top") + dy);
	_SET_INT_FIELD(rect, "bottom", _GET_INT_FIELD(rect, "bottom") + dy);
}

/* ---------------- geometry queries ---------------- */

static void window_offset(struct fw *w, int *x, int *y)
{
	*x = *y = 0;
	for (; w; w = w->parent) {
		*x += w->x;
		*y += w->y;
	}
}

JNIEXPORT jboolean JNICALL Java_android_view_View_native_1getGlobalVisibleRect(JNIEnv *env, jobject self, jlong p, jobject rect)
{
	struct fw *w = _PTR(p);
	int x, y;
	window_offset(w, &x, &y);
	_SET_INT_FIELD(rect, "left", x);
	_SET_INT_FIELD(rect, "top", y);
	_SET_INT_FIELD(rect, "right", x + w->w);
	_SET_INT_FIELD(rect, "bottom", y + w->h);
	return w->visible && w->w > 0 && w->h > 0;
}

JNIEXPORT void JNICALL Java_android_view_View_getWindowVisibleDisplayFrame(JNIEnv *env, jobject self, jobject rect)
{
	_SET_INT_FIELD(rect, "left", 0);
	_SET_INT_FIELD(rect, "top", 0);
	_SET_INT_FIELD(rect, "right", main_window ? main_window->width : 0);
	_SET_INT_FIELD(rect, "bottom", main_window ? main_window->height : 0);
}

JNIEXPORT jboolean JNICALL Java_android_view_View_nativeIsAttachedToWindow(JNIEnv *env, jobject self, jlong p)
{
	struct fw *w = _PTR(p);
	while (w && w->parent)
		w = w->parent;
	return w && main_window && main_window->root == w;
}

JNIEXPORT jboolean JNICALL Java_android_view_View_native_1getMatrix(JNIEnv *env, jobject self, jlong p, jlong m)
{
	struct fw *w = _PTR(p);
	float *mat = _PTR(m);
	memset(mat, 0, 9 * sizeof(float));
	mat[0] = mat[4] = mat[8] = 1;
	mat[2] = w->x;
	mat[5] = w->y;
	return true;
}

JNIEXPORT jboolean JNICALL Java_android_view_View_nativeIsFocused(JNIEnv *env, jclass c, jlong p)
{
	return _PTR(p) == focused_edit;
}

JNIEXPORT void JNICALL Java_android_view_View_nativeRequestFocus(JNIEnv *env, jobject self, jlong p, jint dir)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	if (w && w->kind == FW_EDIT) {
		focused_edit = w;
		flare_request_frame();
	}
}

/* input reaches views through the decor view's dispatchTouchEvent; the
 * natively backed widgets (buttons, text fields) are hit-tested in native
 * code instead, see window.c */
JNIEXPORT jboolean JNICALL Java_android_view_View_native_1gtkEventPropagation(JNIEnv *env, jobject self) { return false; }
JNIEXPORT void JNICALL Java_android_view_View_native_1registerTouchEventPropagation(JNIEnv *env, jobject self, jlong p) {}
JNIEXPORT void JNICALL Java_android_view_View_nativeSetOnTouchListener(JNIEnv *env, jobject self, jlong p) {}
JNIEXPORT void JNICALL Java_android_view_View_nativeSetOnClickListener(JNIEnv *env, jobject self, jlong p) {}
JNIEXPORT void JNICALL Java_android_view_View_nativeSetOnLongClickListener(JNIEnv *env, jobject self, jlong p) {}
JNIEXPORT void JNICALL Java_android_view_View_nativeSetFullscreen(JNIEnv *env, jobject self, jlong p, jboolean f) {}
JNIEXPORT void JNICALL Java_android_view_View_native_1keep_1screen_1on(JNIEnv *env, jclass c, jlong p, jboolean on) {}

JNIEXPORT jobject JNICALL Java_android_view_View_native_1get_1window(JNIEnv *env, jobject self, jlong p)
{
	return main_window && main_window->jwindow ? (*env)->NewLocalRef(env, main_window->jwindow) : NULL;
}

struct fw *fw_hit(struct fw *w, int x, int y, int *lx, int *ly)
{
	if (!w->visible || x < 0 || y < 0 || x >= w->w || y >= w->h)
		return NULL;
	for (int i = w->nkids - 1; i >= 0; i--) {
		struct fw *c = w->kids[i];
		struct fw *hit = fw_hit(c, x - c->x, y - c->y, lx, ly);
		if (hit)
			return hit;
	}
	*lx = x;
	*ly = y;
	return w;
}

/* ---------------- TextView ---------------- */

static struct fw *text_new(JNIEnv *env, jobject self, int kind)
{
	struct fw *w = fw_new(env, self, kind);
	return w;
}

JNIEXPORT jlong JNICALL Java_android_widget_TextView_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	return _INTPTR(text_new(env, self, FW_TEXT));
}

JNIEXPORT void JNICALL Java_android_widget_TextView_native_1setText(JNIEnv *env, jobject self, jstring text)
{
	struct fw *w = fw_of(env, self);
	if (!w)
		return;
	free(w->text);
	w->text = dup_jstring(env, text);
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_widget_TextView_native_1set_1markup(JNIEnv *env, jobject self, jint on)
{
	/* Spanned text arrives as plain text here: no Pango markup to parse */
}

JNIEXPORT void JNICALL Java_android_widget_TextView_native_1setTextColor(JNIEnv *env, jobject self, jint color)
{
	struct fw *w = fw_of(env, self);
	if (w) {
		w->color = color;
		flare_request_frame();
	}
}

JNIEXPORT void JNICALL Java_android_widget_TextView_setTextSize(JNIEnv *env, jobject self, jfloat size)
{
	struct fw *w = fw_of(env, self);
	if (w) {
		w->text_size = size;
		flare_request_layout();
	}
}

JNIEXPORT void JNICALL Java_android_widget_TextView_native_1setCompoundDrawables(JNIEnv *env, jobject self, jlong p, jlong l, jlong t, jlong r, jlong b)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	jlong v[4] = {l, t, r, b};
	for (int i = 0; i < 4; i++) {
		fp_unref(w->compound[i]);
		w->compound[i] = fp_ref(_PTR(v[i]));
	}
	flare_request_layout();
}

/* ---------------- Button ---------------- */

JNIEXPORT jlong JNICALL Java_android_widget_Button_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	return _INTPTR(text_new(env, self, FW_BUTTON));
}

JNIEXPORT void JNICALL Java_android_widget_Button_native_1setText(JNIEnv *env, jobject self, jlong p, jstring text)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	free(w->text);
	w->text = dup_jstring(env, text);
	flare_request_layout();
}

JNIEXPORT jobject JNICALL Java_android_widget_Button_getText(JNIEnv *env, jobject self)
{
	struct fw *w = fw_of(env, self);
	return (*env)->NewStringUTF(env, w && w->text ? w->text : "");
}

JNIEXPORT void JNICALL Java_android_widget_Button_nativeSetOnClickListener(JNIEnv *env, jobject self, jlong p) {}

JNIEXPORT void JNICALL Java_android_widget_Button_native_1setCompoundDrawables(JNIEnv *env, jobject self, jlong p, jlong paintable)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	fp_unref(w->compound[0]);
	w->compound[0] = fp_ref(_PTR(paintable));
	flare_request_layout();
}

/* ---------------- EditText ---------------- */

struct fw *focused_edit;

JNIEXPORT jlong JNICALL Java_android_widget_EditText_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	struct fw *w = text_new(env, self, FW_EDIT);
	w->text = strdup("");
	return _INTPTR(w);
}

JNIEXPORT jstring JNICALL Java_android_widget_EditText_native_1getText(JNIEnv *env, jobject self, jlong p)
{
	struct fw *w = _PTR(p);
	return (*env)->NewStringUTF(env, w->text ? w->text : "");
}

static void text_changed(JNIEnv *env, struct fw *w)
{
	for (int i = 0; i < w->nwatchers; i++) {
		jstring js = (*env)->NewStringUTF(env, w->text);
		jobject ed = J_new__SpannableStringBuilder(env, js);
		J__TextWatcher__onTextChanged(env, w->watchers[i], ed, 0, 0, (jint)strlen(w->text));
		if ((*env)->ExceptionCheck(env))
			{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
		J__TextWatcher__afterTextChanged(env, w->watchers[i], ed);
		if ((*env)->ExceptionCheck(env))
			{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
		(*env)->DeleteLocalRef(env, ed);
		(*env)->DeleteLocalRef(env, js);
	}
	flare_request_frame();
}

JNIEXPORT void JNICALL Java_android_widget_EditText_native_1setText(JNIEnv *env, jobject self, jlong p, jstring text)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	free(w->text);
	w->text = dup_jstring(env, text);
	if (!w->text)
		w->text = strdup("");
	text_changed(env, w);
}

JNIEXPORT void JNICALL Java_android_widget_EditText_native_1setHint(JNIEnv *env, jobject self, jlong p, jstring text)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	free(w->hint);
	w->hint = dup_jstring(env, text);
	flare_request_frame();
}

JNIEXPORT jobject JNICALL Java_android_widget_EditText_native_1getHint(JNIEnv *env, jobject self, jlong p)
{
	struct fw *w = _PTR(p);
	return (*env)->NewStringUTF(env, w->hint ? w->hint : "");
}

JNIEXPORT void JNICALL Java_android_widget_EditText_native_1addTextChangedListener(JNIEnv *env, jobject self, jlong p, jobject l)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	if (w->nwatchers < 8)
		w->watchers[w->nwatchers++] = (*env)->NewGlobalRef(env, l);
}

JNIEXPORT void JNICALL Java_android_widget_EditText_native_1removeTextChangedListener(JNIEnv *env, jobject self, jlong p, jobject l)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	for (int i = 0; i < w->nwatchers; i++)
		if ((*env)->IsSameObject(env, w->watchers[i], l)) {
			(*env)->DeleteGlobalRef(env, w->watchers[i]);
			w->watchers[i] = w->watchers[--w->nwatchers];
			break;
		}
}

JNIEXPORT void JNICALL Java_android_widget_EditText_native_1setOnEditorActionListener(JNIEnv *env, jobject self, jlong p, jobject l)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	if (w->editor_action)
		(*env)->DeleteGlobalRef(env, w->editor_action);
	w->editor_action = l ? (*env)->NewGlobalRef(env, l) : NULL;
}

/* a key typed into the focused text field: utf8 text, or a keysym for
 * BackSpace (0xff08) and Return (0xff0d) */
void fw_edit_key(JNIEnv *env, struct fw *w, const char *utf8, int keysym)
{
	size_t n = strlen(w->text);
	if (keysym == 0xff08) {
		if (!n)
			return;
		char *e = g_utf8_find_prev_char(w->text, w->text + n);
		*(e ? e : w->text) = 0;
	} else if (keysym == 0xff0d) {
		if (w->editor_action) {
			jobject v = (*env)->NewLocalRef(env, w->view);
			J__TextView__OnEditorActionListener__onEditorAction(env, w->editor_action, v, 6 /* IME_ACTION_DONE */, NULL);
			if ((*env)->ExceptionCheck(env))
				{ (*env)->ExceptionDescribe(env); (*env)->ExceptionClear(env); }
		}
		return;
	} else if (utf8 && *utf8) {
		w->text = realloc(w->text, n + strlen(utf8) + 1);
		strcpy(w->text + n, utf8);
	} else {
		return;
	}
	text_changed(env, w);
}

/* ---------------- ImageView / ImageButton ---------------- */

JNIEXPORT jlong JNICALL Java_android_widget_ImageView_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	return _INTPTR(fw_new(env, self, FW_IMAGE));
}

JNIEXPORT void JNICALL Java_android_widget_ImageView_native_1setDrawable(JNIEnv *env, jobject self, jlong p, jlong paintable)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	fp_unref(w->image);
	w->image = fp_ref(_PTR(paintable));
	flare_request_layout();
}

JNIEXPORT void JNICALL Java_android_widget_ImageView_native_1setScaleType(JNIEnv *env, jobject self, jlong p, jint t)
{
	((struct fw *)_PTR(p))->scale_type = t;
	flare_request_frame();
}

JNIEXPORT jlong JNICALL Java_android_widget_ImageButton_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	return _INTPTR(fw_new(env, self, FW_IMAGE));
}

JNIEXPORT void JNICALL Java_android_widget_ImageButton_native_1setDrawable(JNIEnv *env, jobject self, jlong p, jlong paintable)
{
	Java_android_widget_ImageView_native_1setDrawable(env, self, p, paintable);
}

JNIEXPORT void JNICALL Java_android_widget_ImageButton_nativeSetOnClickListener(JNIEnv *env, jobject self, jlong p) {}

/* ---------------- ScrollView ---------------- */

JNIEXPORT jlong JNICALL Java_android_widget_ScrollView_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	return _INTPTR(fw_new(env, self, FW_SCROLL));
}

JNIEXPORT void JNICALL Java_android_widget_ScrollView_native_1addView(JNIEnv *env, jobject self, jlong p, jlong child, jint index, jobject lp)
{
	Java_android_view_ViewGroup_native_1addView(env, self, p, child, index, lp);
}

JNIEXPORT void JNICALL Java_android_widget_ScrollView_native_1removeView(JNIEnv *env, jobject self, jlong p, jlong child)
{
	Java_android_view_ViewGroup_native_1removeView(env, self, p, child);
}

/* ---------------- SurfaceView: software canvases only ---------------- */

jlong flare_generic_widget(JNIEnv *env, jobject view)
{
	return Java_android_view_View_native_1constructor(env, view, NULL, NULL);
}

JNIEXPORT jlong JNICALL Java_android_view_SurfaceView_native_1constructor(JNIEnv *env, jobject self, jobject ctx, jobject attrs)
{
	return _INTPTR(fw_new(env, self, FW_IMAGE));
}

/* lockCanvas: a canvas on a fresh image of the view's size */
JNIEXPORT jlong JNICALL Java_android_view_SurfaceView_native_1createSnapshot(JNIEnv *env, jobject self)
{
	struct fw *w = fw_of(env, self);
	int width = w && w->w > 0 ? w->w : 1, height = w && w->h > 0 ? w->h : 1;
	return _INTPTR(fc_new(cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width, height)));
}

/* unlockCanvasAndPost: the image becomes the view's content */
JNIEXPORT void JNICALL Java_android_view_SurfaceView_native_1postSnapshot(JNIEnv *env, jobject self, jlong p, jlong snap)
{
	struct fw *w = _PTR(p);
	if (!w)
		return;
	struct fcanvas *c = _PTR(snap);
	cairo_surface_flush(c->surface);
	struct fpaintable *img = fp_new(FP_SURFACE);
	img->surface = cairo_surface_reference(c->surface);
	fc_free(env, c);
	if (w) {
		fp_unref(w->image);
		w->image = img;
		w->scale_type = 1;
	} else {
		fp_unref(img);
	}
	flare_request_frame();
}

/* ---------------- ViewTreeObserver ---------------- */

bool have_global_layout_listeners, have_predraw_listeners;

JNIEXPORT void JNICALL Java_android_view_ViewTreeObserver_native_1set_1have_1global_1layout_1listeners(JNIEnv *env, jobject self, jboolean on)
{
	have_global_layout_listeners = on;
}

JNIEXPORT void JNICALL Java_android_view_ViewTreeObserver_native_1set_1have_1predraw_1listeners(JNIEnv *env, jobject self, jboolean on)
{
	have_predraw_listeners = on;
}
