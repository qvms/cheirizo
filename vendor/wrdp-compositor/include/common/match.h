/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_MATCH_H
#define WRDP_COMPOSITOR_MATCH_H

#include <stdbool.h>

/**
 * match_glob() - Pattern match using shell wildcard rules (see glob(7))
 * @pattern: Pattern to match against.
 * @string: String to search.
 * Note: Comparison case-insensitive.
 */
bool match_glob(const char *pattern, const char *string);

#endif /* WRDP_COMPOSITOR_MATCH_H */
