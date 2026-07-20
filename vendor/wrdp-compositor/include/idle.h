/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_IDLE_H
#define WRDP_COMPOSITOR_IDLE_H

struct wl_display;
struct wlr_seat;

void idle_manager_create(struct wl_display *display, struct wlr_seat *wlr_seat);
void idle_manager_notify_activity(struct wlr_seat *seat);

#endif /* WRDP_COMPOSITOR_IDLE_H */
