/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_MAGNIFIER_H
#define WRDP_COMPOSITOR_MAGNIFIER_H

#include <stdbool.h>

struct server;
struct output;
struct wlr_buffer;
struct wlr_box;

enum magnify_dir {
	MAGNIFY_INCREASE,
	MAGNIFY_DECREASE
};

void magnify_toggle(struct server *server);
void magnify_set_scale(struct server *server, enum magnify_dir dir);
bool output_wants_magnification(struct output *output);
void magnify(struct output *output, struct wlr_buffer *output_buffer,
	struct wlr_box *damage);
bool is_magnify_on(void);
void magnify_reset(void);

#endif /* WRDP_COMPOSITOR_MAGNIFIER_H */
