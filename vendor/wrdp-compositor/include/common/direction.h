/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_DIRECTION_H
#define WRDP_COMPOSITOR_DIRECTION_H

#include "view.h"

enum wlr_direction direction_from_view_edge(enum view_edge edge);
enum wlr_direction direction_get_opposite(enum wlr_direction direction);

#endif /* WRDP_COMPOSITOR_DIRECTION_H */
