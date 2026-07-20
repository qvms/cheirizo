/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_GESTURES_H
#define WRDP_COMPOSITOR_GESTURES_H

struct seat;

void gestures_init(struct seat *seat);
void gestures_finish(struct seat *seat);

#endif /* WRDP_COMPOSITOR_GESTURES_H */
