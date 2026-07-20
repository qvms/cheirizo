/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_DND_H
#define WRDP_COMPOSITOR_DND_H

#include <wayland-server-core.h>

struct seat;

void dnd_init(struct seat *seat);
void dnd_icons_show(struct seat *seat, bool show);
void dnd_icons_move(struct seat *seat, double x, double y);
void dnd_finish(struct seat *seat);

#endif /* WRDP_COMPOSITOR_DND_H */
