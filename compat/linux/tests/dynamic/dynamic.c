/* dynamic.c - dynamic linking on Flare: musl's ld-musl-aarch64.so.1 maps
 * this program's libraries with file-backed mmap, relocates them and
 * starts it; dlopen/dlsym load a plugin later. Prints "dynamic: ok". */
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

extern int greet_calls;
const char *greet(void);

int main(void) {
    int fails = 0;
    const char *g = greet();
    int ok = !strcmp(g, "hello from libgreet.so") && greet_calls == 1;
    printf("dynamic: linked library: %s\n", ok ? "ok" : "FAILED");
    fails += !ok;

    void *h = dlopen("/usr/lib/plugin.so", RTLD_NOW);
    int (*sq)(int) = h ? (int (*)(int))dlsym(h, "plugin_square") : 0;
    ok = sq && sq(12) == 144 && greet_calls == 2;
    printf("dynamic: dlopen/dlsym: %s\n", ok ? "ok" : h ? "FAILED" : dlerror());
    fails += !ok;

    printf("dynamic: %s\n", fails ? "FAILED" : "ok");
    return fails ? 1 : 0;
}
