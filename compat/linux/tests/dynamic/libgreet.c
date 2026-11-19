/* libgreet.so - a shared library the dynamic test links against. */
int greet_calls;
const char *greet(void) { greet_calls++; return "hello from libgreet.so"; }
