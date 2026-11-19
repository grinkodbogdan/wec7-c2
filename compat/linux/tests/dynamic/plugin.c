/* plugin.so - loaded at run time with dlopen. It calls back into
 * libgreet.so, which the program already loaded. */
extern int greet_calls;
const char *greet(void);
int plugin_square(int x) { greet(); return x * x; }
