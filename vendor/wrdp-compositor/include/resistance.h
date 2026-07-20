/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_RESISTANCE_H
#define WRDP_COMPOSITOR_RESISTANCE_H
#include "wrdp-compositor.h"

/**
 * resistance_unsnap_apply() - Apply resistance when dragging a
 * maximized/tiled window. Returns true when the view needs to be un-tiled.
 */
bool resistance_unsnap_apply(struct view *view, int *x, int *y);
void resistance_move_apply(struct view *view, int *x, int *y);
void resistance_resize_apply(struct view *view, struct wlr_box *new_view_geo);

#endif /* WRDP_COMPOSITOR_RESISTANCE_H */
