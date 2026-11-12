#include <sys/caps.h>
#define	PRIV_DRIVER	0
#define	priv_check(td, p)	caps_priv_check_td((td), SYSCAP_RESTRICTEDROOT)
