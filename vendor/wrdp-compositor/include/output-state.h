/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_OUTPUT_STATE_H
#define WRDP_COMPOSITOR_OUTPUT_STATE_H

#include <stdbool.h>

struct output;

void output_state_init(struct output *output);

bool output_state_commit(struct output *output);

#endif // WRDP_COMPOSITOR_OUTPUT_STATE_H
