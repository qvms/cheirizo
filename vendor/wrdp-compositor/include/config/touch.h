/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_TOUCH_CONFIG_H
#define WRDP_COMPOSITOR_TOUCH_CONFIG_H

#include <stdint.h>
#include <wayland-util.h>

struct touch_config_entry {
	char *device_name;
	char *output_name;
	bool force_mouse_emulation;

	struct wl_list link;     /* struct rcxml.touch_configs */
};

struct touch_config_entry *touch_find_config_for_device(char *device_name);

#endif /* WRDP_COMPOSITOR_TOUCH_CONFIG_H */
