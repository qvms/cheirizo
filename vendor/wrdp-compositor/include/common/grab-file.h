/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
/*
 * Read file into memory
 *
 * Copyright Johan Malm 2020
 */

#ifndef WRDP_COMPOSITOR_GRAB_FILE_H
#define WRDP_COMPOSITOR_GRAB_FILE_H

#include "common/buf.h"

/**
 * grab_file - read file into memory buffer
 * @filename: file to read
 * Free returned buffer with buf_reset().
 */
struct buf grab_file(const char *filename);

#endif /* WRDP_COMPOSITOR_GRAB_FILE_H */
