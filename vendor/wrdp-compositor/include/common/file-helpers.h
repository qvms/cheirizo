/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_FILE_HELPERS_H
#define WRDP_COMPOSITOR_FILE_HELPERS_H
#include <stdbool.h>

/**
 * file_exists() - Test if file exists.
 * @filename: Name of file to test.
 */
bool file_exists(const char *filename);

#endif /* WRDP_COMPOSITOR_FILE_HELPERS_H */
