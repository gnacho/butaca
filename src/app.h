#ifndef BUTACA_APP_H
#define BUTACA_APP_H
#include <stdio.h>                 /* FILE */

/* (1) config.local.h (gitignored, dev-only) overrides the host placeholder. NO token macro:
 *     the binary carries no credentials — server access comes from the signed-in session, or from
 *     the /tmp/butaca-token dev trigger for automated runs (the token in config.local.h is read by
 *     tests/run.py on the HOST, never compiled in). */
#if defined(__has_include)
#  if __has_include("config.local.h")
#    include "config.local.h"
#  endif
#endif
/* A config.local.h written before the rename still defines the old macros. Honour them, so an
 * existing checkout keeps building without editing a gitignored file. */
#ifndef SERVER_HOST
#  ifdef PMS_HOST
#    define SERVER_HOST PMS_HOST
#  else
#    define SERVER_HOST "YOUR_SERVER_HOST"
#  endif
#endif
#ifndef SERVER_PORT
#  ifdef PMS_PORT
#    define SERVER_PORT PMS_PORT
#  else
#    define SERVER_PORT 32400
#  endif
#endif

/* (2) the one process-wide global: the event/diagnostic log, DEFINED once as
 *     FILE *elogf = NULL; in main.c and read by nearly every UI/glue module. */
extern FILE *elogf;

#endif /* BUTACA_APP_H */
