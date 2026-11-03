#ifndef BUTACA_ASS_H
#define BUTACA_ASS_H

/* Private, versioned facade over the pinned bundled libass. No upstream struct
 * crosses this ABI; every handle and returned pixel buffer belongs to one worker. */
#include <stddef.h>
#include <stdint.h>

typedef struct ButacaAss ButacaAss;
typedef struct {
    int x, y, width, height;
    size_t bytes;
    const uint8_t *rgba; /* straight RGBA; valid until the next render or destroy */
} ButacaAssBitmap;

#define BUTACA_ASS_MAX_REGIONS 64
typedef struct {
    size_t count;
    const ButacaAssBitmap *regions; /* disjoint bounds; same lifetime as their pixels */
} ButacaAssFrame;

unsigned butaca_ass_abi_version(void);
ButacaAss *butaca_ass_create(const char *default_font);
void butaca_ass_destroy(ButacaAss *ctx);
int butaca_ass_add_font(ButacaAss *ctx, const char *name, const uint8_t *data, size_t len);
int butaca_ass_load(ButacaAss *ctx, const uint8_t *data, size_t len, int script);
int butaca_ass_chunk(ButacaAss *ctx, const uint8_t *data, size_t len,
                  int64_t start_ms, int64_t duration_ms);
void butaca_ass_prune_before(ButacaAss *ctx, int64_t deadline_ms);
void butaca_ass_flush_events(ButacaAss *ctx);
/* -1: refused/failed, 0: identical output (frame unchanged), 1: changed output.
 * A changed frame with count=0 clears the old subtitle. */
int butaca_ass_render(ButacaAss *ctx, int64_t now_ms, int width, int height,
                   int storage_width, int storage_height,
                   ButacaAssFrame *frame);

#endif
