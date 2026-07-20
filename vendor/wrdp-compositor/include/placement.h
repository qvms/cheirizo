/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_PLACEMENT_H
#define WRDP_COMPOSITOR_PLACEMENT_H

#include <stdbool.h>
#include <wlr/util/box.h>
#include "view.h"

bool placement_find_best(struct view *view, struct wlr_box *geometry);

#endif /* WRDP_COMPOSITOR_PLACEMENT_H */
