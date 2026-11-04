#ifndef BUTACA_APP_H
#define BUTACA_APP_H
#include <stdio.h>                 /* FILE */

/* (1) no server macro lives here. The binary carries no credentials and no server address: the
 *     app learns its Jellyfin URL from the signed-in session's config file. The token in
 *     config.local.h is read by tests/run.py on the HOST, never compiled in. */

/* (2) the one process-wide global: the event/diagnostic log, DEFINED once as
 *     FILE *elogf = NULL; in main.c and read by nearly every UI/glue module. */
extern FILE *elogf;

#endif /* BUTACA_APP_H */
