/* atl-run: start an Android app on ART with ATL's framework, drawing into
 * an Arcan window. The launch sequence follows ATL's main-executable/main.c
 * (create_vm, Context.createApplication, Activity.createMainActivity), with
 * the GTK application replaced by Flare's window.
 *
 *   atl-run [-l ACTIVITY] [-w WIDTH] [-h HEIGHT] APP.apk
 *
 * SPDX-License-Identifier: GPL-3.0-or-later (derived from ATL) */
#include <errno.h>
#include <glib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "flare.h"
#include "util.h"

#ifndef ATL_PREFIX
#define ATL_PREFIX "/opt/atl"
#endif

JavaVM *jvm;
char *apk_path;
static char *app_data_dir;
jobject application_object;

char *get_app_data_dir(void)
{
	return app_data_dir;
}

void activity_start(JNIEnv *env, jobject activity_object);

static void check(JNIEnv *env, const char *what)
{
	if ((*env)->ExceptionCheck(env)) {
		fprintf(stderr, "atl: exception in %s:\n", what);
		(*env)->ExceptionDescribe(env);
		(*env)->ExceptionClear(env);
	}
}

static JNIEnv *create_vm(const char *apk, const char *natives, const char *app_lib)
{
	JNIEnv *env;
	const char *bcp = getenv("ATL_BOOTCLASSPATH");
	char *opts[] = {
		g_strdup_printf("-Djava.library.path=%s:%s", natives, app_lib),
		g_strdup_printf("-Djava.class.path=%s/api-impl.jar:%s/framework-res.apk", ATL_PREFIX, ATL_PREFIX),
		g_strdup_printf("-Datl.app.class.path=%s", apk),
		g_strdup_printf("-Datl.app.library.path=%s:%s", natives, app_lib),
		bcp ? g_strdup_printf("-Xbootclasspath:%s", bcp) : NULL,
		"-Xnoimage-dex2oat",
		"-Xusejit:false",
		"-Xms32m",
		"-Xmx384m",
	};
	JavaVMOption options[G_N_ELEMENTS(opts)];
	int n = 0;
	for (size_t i = 0; i < G_N_ELEMENTS(opts); i++)
		if (opts[i])
			options[n++].optionString = opts[i];
	JavaVMInitArgs args = {.version = JNI_VERSION_1_6, .nOptions = n, .options = options, .ignoreUnrecognized = JNI_FALSE};
	if (JNI_CreateJavaVM(&jvm, (void **)&env, &args) < 0) {
		fprintf(stderr, "atl: unable to start ART\n");
		exit(1);
	}
	return env;
}

int main(int argc, char **argv)
{
	const char *activity = NULL;
	int width = 0, height = 0, opt;
	while ((opt = getopt(argc, argv, "l:w:h:")) != -1) {
		switch (opt) {
			case 'l': activity = optarg; break;
			case 'w': width = atoi(optarg); break;
			case 'h': height = atoi(optarg); break;
			default:
				fprintf(stderr, "usage: atl-run [-l ACTIVITY] [-w WIDTH] [-h HEIGHT] APP.apk\n");
				return 2;
		}
	}
	if (optind >= argc) {
		fprintf(stderr, "usage: atl-run [-l ACTIVITY] [-w WIDTH] [-h HEIGHT] APP.apk\n");
		return 2;
	}
	char *apk = realpath(argv[optind], NULL);
	if (!apk) {
		fprintf(stderr, "atl: %s: %s\n", argv[optind], strerror(errno));
		return 1;
	}
	apk_path = apk;

	/* per-app data under $ANDROID_APP_DATA_DIR (default /data/data) */
	const char *base = getenv("ANDROID_APP_DATA_DIR");
	if (!base)
		base = "/data/data";
	mkdir(base, 0755);
	char *name = g_path_get_basename(apk);
	app_data_dir = g_strdup_printf("%s/%s_/", base, name);
	mkdir(app_data_dir, 0755);
	char *app_lib = g_strdup_printf("%slib", app_data_dir);
	mkdir(app_lib, 0755);
	char *natives = g_strdup_printf("%s/lib", ATL_PREFIX);

	if (!width || !height) {
		width = width ? width : 480;
		height = height ? height : 800;
	}
	flare_window_init(&width, &height, name);

	JNIEnv *env = create_vm(apk, natives, app_lib);
	(*env)->GetJavaVM(env, &jvm);

	jclass display = _CACHED_CLASS(Display);
	_SET_STATIC_INT_FIELD(display, "window_width", width);
	_SET_STATIC_INT_FIELD(display, "window_height", height);

	/* register our JNI library with the class loader of the framework */
	jobject loader = J__Class__getClassLoader(env, _CACHED_CLASS(View));
	J__Runtime__loadLibrary(env, J__Runtime__getRuntime(env), _JSTRING("translation_layer_main"), loader);
	check(env, "loadLibrary");

	prepare_main_looper(env);

	application_object = J__Context__createApplication(env, _INTPTR(main_window));
	check(env, "createApplication");
	application_object = _REF(application_object);
	J__ContentProvider__createContentProviders(env);
	check(env, "createContentProviders");
	J__Application__onCreate(env, application_object);
	check(env, "Application.onCreate");

	jobject a = J__Activity__createMainActivity(env, activity ? _JSTRING(activity) : NULL, _INTPTR(main_window), NULL);
	check(env, "createMainActivity");
	if (!a)
		return 1;
	activity_start(env, a);
	check(env, "activity start");

	flare_run(env);
	return 0;
}
