/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_FD_UTIL_H
#define WRDP_COMPOSITOR_FD_UTIL_H

void increase_nofile_limit(void);
void restore_nofile_limit(void);

#endif /* WRDP_COMPOSITOR_FD_UTIL_H */
