/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_TABLET_TOOL_CONFIG_H
#define WRDP_COMPOSITOR_TABLET_TOOL_CONFIG_H

#include <stdint.h>

enum motion {
	LAB_TABLET_MOTION_ABSOLUTE = 0,
	LAB_TABLET_MOTION_RELATIVE,
};

enum motion tablet_parse_motion(const char *name);

#endif /* WRDP_COMPOSITOR_TABLET_TOOL_CONFIG_H */
