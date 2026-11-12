/* No alq(9) on DragonFly: athp's packet tracing stays off. */
struct alq;
#define	ALQ_DEFAULT_CMODE	0
#define	alq_open(alqp, path, cred, cmode, size, flags)	(EOPNOTSUPP)
#define	alq_close(alq)		do { } while (0)
