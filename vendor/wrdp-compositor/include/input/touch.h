/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_TOUCH_H
#define WRDP_COMPOSITOR_TOUCH_H

struct seat;

void touch_init(struct seat *seat);
void touch_finish(struct seat *seat);

#endif /* WRDP_COMPOSITOR_TOUCH_H */
