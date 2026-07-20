/* SPDX-License-Identifier: GPL-2.0-only */
/* WRDP modifications, 2026. */
#ifndef WRDP_COMPOSITOR_INPUT_H
#define WRDP_COMPOSITOR_INPUT_H

struct seat;

void input_handlers_init(struct seat *seat);
void input_handlers_finish(struct seat *seat);

#endif /* WRDP_COMPOSITOR_INPUT_H */
