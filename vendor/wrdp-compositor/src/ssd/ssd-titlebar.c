// SPDX-License-Identifier: GPL-2.0-only
/* WRDP modifications, 2026. */

#define _POSIX_C_SOURCE 200809L
#include <assert.h>
#include <string.h>
#include "buffer.h"
#include "config.h"
#include "common/mem.h"
#include "common/scaled-font-buffer.h"
#include "common/scaled-icon-buffer.h"
#include "common/scaled-img-buffer.h"
#include "common/scene-helpers.h"
#include "common/string-helpers.h"
#include "desktop-entry.h"
#include "img/img.h"
#include "wrdp-compositor.h"
#include "node.h"
#include "ssd-internal.h"
#include "theme.h"
#include "view.h"

#define FOR_EACH_STATE(ssd, tmp) FOR_EACH(tmp, \
	&(ssd)->titlebar.active, \
	&(ssd)->titlebar.inactive)

static void set_squared_corners(struct ssd *ssd, bool enable);
static void set_alt_button_icon(struct ssd *ssd, enum ssd_part_type type, bool enable);
static void update_visible_buttons(struct ssd *ssd);

static float
os9_view_scale(struct view *view)
{
	if (view && view->output && view->output->wlr_output
			&& view->output->wlr_output->scale > 0) {
		return view->output->wlr_output->scale;
	}
	return 1.0f;
}

static bool
os9_platinum_theme_enabled(void)
{
	return rc.theme_name && (strstr(rc.theme_name, "Platinum")
		|| strstr(rc.theme_name, "Mac-OS-9")
		|| strstr(rc.theme_name, "MacOS9"));
}

static float
os9_platinum_client_scale(void)
{
	const char *client = getenv("WRDP_COMPOSITOR_OS9_CLIENT_SCALE");
	if (client && *client) {
		char *end = NULL;
		float parsed = strtof(client, &end);
		if (end != client && parsed >= 1.0f && parsed <= 3.0f) {
			return parsed;
		}
	}
	return 1.5f;
}

static int
os9_tile_metric(int value)
{
	return MAX(1, (int)((float)value * os9_platinum_client_scale() + 0.5f));
}

static int
os9_line_metric(void)
{
	return os9_tile_metric(1);
}

static double
os9_rendered_pixel(void)
{
	return 1.0;
}

static void
set_cairo_rgb(cairo_t *cairo, uint32_t rgb)
{
	cairo_set_source_rgb(cairo,
		((rgb >> 16) & 0xff) / 255.0,
		((rgb >> 8) & 0xff) / 255.0,
		(rgb & 0xff) / 255.0);
}

static cairo_surface_t *
os9_platinum_atlas_surface(void)
{
	static cairo_surface_t *atlas = NULL;
	static bool tried = false;
	if (tried) {
		return atlas;
	}
	tried = true;
	const char *path = getenv("WRDP_COMPOSITOR_OS9_ATLAS");
	if (!path || !*path) {
		path = "/usr/share/themes/PlatinumTheme-wrdp-compositor/openbox-3/platinum-atlas.png";
	}
	atlas = cairo_image_surface_create_from_png(path);
	if (cairo_surface_status(atlas) != CAIRO_STATUS_SUCCESS) {
		cairo_surface_destroy(atlas);
		atlas = NULL;
	}
	return atlas;
}

static bool
os9_draw_atlas(cairo_t *cairo, int sx, int sy, int sw, int sh,
		int dx, int dy, int dw, int dh)
{
	cairo_surface_t *atlas = os9_platinum_atlas_surface();
	if (!atlas || sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0) {
		return false;
	}
	cairo_save(cairo);
	cairo_rectangle(cairo, dx, dy, dw, dh);
	cairo_clip(cairo);
	double xs = (double)dw / (double)sw;
	double ys = (double)dh / (double)sh;
	cairo_translate(cairo, dx - sx * xs, dy - sy * ys);
	cairo_scale(cairo, xs, ys);
	cairo_set_source_surface(cairo, atlas, 0, 0);
	cairo_pattern_set_filter(cairo_get_source(cairo), CAIRO_FILTER_NEAREST);
	cairo_set_operator(cairo, CAIRO_OPERATOR_OVER);
	cairo_paint(cairo);
	cairo_restore(cairo);
	return true;
}

static void
os9_tile_atlas_scaled_x(cairo_t *cairo, int sx, int sy, int sw, int sh,
		int dx, int dy, int dw, int dh)
{
	int tile_w = os9_tile_metric(sw);
	for (int x = dx; x < dx + dw; x += tile_w) {
		int tw = MIN(tile_w, dx + dw - x);
		int src_w = MAX(1, (int)((double)sw * tw / tile_w + 0.5));
		os9_draw_atlas(cairo, sx, sy, src_w, sh, x, dy, tw, dh);
	}
}

static cairo_surface_t *
os9_create_resampled_tile(int sx, int sy, int sw, int sh, int out_w, int out_h)
{
	cairo_surface_t *atlas = os9_platinum_atlas_surface();
	if (!atlas || out_w <= 0 || out_h <= 0) {
		return NULL;
	}
	/* Build at 2x first, then resample down. This avoids the visible phase/glitch
	 * caused by repeatedly scaling clipped atlas slices in the stripe band. */
	int hi_w = out_w * 2;
	int hi_h = out_h * 2;
	cairo_surface_t *hi = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, hi_w, hi_h);
	cairo_t *cr = cairo_create(hi);
	cairo_set_operator(cr, CAIRO_OPERATOR_CLEAR);
	cairo_paint(cr);
	cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
	cairo_rectangle(cr, 0, 0, hi_w, hi_h);
	cairo_clip(cr);
	cairo_scale(cr, (double)hi_w / (double)sw, (double)hi_h / (double)sh);
	cairo_set_source_surface(cr, atlas, -sx, -sy);
	cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_NEAREST);
	cairo_paint(cr);
	cairo_destroy(cr);

	cairo_surface_t *tile = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, out_w, out_h);
	cr = cairo_create(tile);
	cairo_set_operator(cr, CAIRO_OPERATOR_CLEAR);
	cairo_paint(cr);
	cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
	cairo_scale(cr, 0.5, 0.5);
	cairo_set_source_surface(cr, hi, 0, 0);
	cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
	cairo_paint(cr);
	cairo_destroy(cr);
	cairo_surface_destroy(hi);
	return tile;
}

static void
os9_fill_resampled_tile_x(cairo_t *cairo, int sx, int sy, int sw, int sh,
		int dx, int dy, int dw, int dh)
{
	int tile_w = os9_tile_metric(sw);
	int tile_h = dh;
	cairo_surface_t *tile = os9_create_resampled_tile(sx, sy, sw, sh, tile_w, tile_h);
	if (!tile) {
		os9_tile_atlas_scaled_x(cairo, sx, sy, sw, sh, dx, dy, dw, dh);
		return;
	}
	cairo_save(cairo);
	cairo_rectangle(cairo, dx, dy, dw, dh);
	cairo_clip(cairo);
	cairo_pattern_t *pattern = cairo_pattern_create_for_surface(tile);
	cairo_pattern_set_extend(pattern, CAIRO_EXTEND_REPEAT);
	cairo_pattern_set_filter(pattern, CAIRO_FILTER_NEAREST);
	cairo_matrix_t matrix;
	cairo_matrix_init_translate(&matrix, -dx, -dy);
	cairo_pattern_set_matrix(pattern, &matrix);
	cairo_set_source(cairo, pattern);
	cairo_paint(cairo);
	cairo_pattern_destroy(pattern);
	cairo_restore(cairo);
	cairo_surface_destroy(tile);
}

/* Same smooth Platinum bevel used by the window frame borders so the titlebar
 * side borders match the frame exactly (highlight against the light-facing edge
 * fading to a dark inner edge). */
static void
os9_titlebar_fill_bevel(cairo_t *cairo, double bx0, double by0, double bx1, double by1)
{
	cairo_pattern_t *pat = cairo_pattern_create_linear(bx0, by0, bx1, by1);
	/* Titlebar side borders: white highlight -> light grey only. We drop the
	 * medium-grey inner band the frame borders use, because against the light
	 * pinstripes it would read as a dark vertical "inset" line intruding into
	 * the titlebar. The light grey blends straight into the stripes. */
	cairo_pattern_add_color_stop_rgb(pat, 0.00, 1.00, 1.00, 1.00);
	cairo_pattern_add_color_stop_rgb(pat, 0.28, 1.00, 1.00, 1.00);
	cairo_pattern_add_color_stop_rgb(pat, 0.2801, 0.808, 0.808, 0.808);
	cairo_pattern_add_color_stop_rgb(pat, 1.00, 0.808, 0.808, 0.808);
	cairo_set_source(cairo, pat);
	cairo_fill(cairo);
	cairo_pattern_destroy(pat);
}

static bool
os9_render_titlebar_from_atlas(cairo_t *cairo, int width, int height,
		bool active, int title_x, int title_w)
{
	if (!os9_platinum_atlas_surface()) {
		return false;
	}
	cairo_set_operator(cairo, CAIRO_OPERATOR_CLEAR);
	cairo_paint(cairo);
	cairo_set_operator(cairo, CAIRO_OPERATOR_OVER);

	double olines = os9_rendered_pixel();
	int oli = (int)(olines + 0.5);
	int tlw = os9_tile_metric(21);
	int trw = os9_tile_metric(36);
	int left = rc.theme->border_width;
	int client_w = MAX(1, width - 2 * left);
	int title_area_x = MIN(tlw, width);
	int title_area_w = MAX(0, width - tlw - trw);

	/* Base: one coherent XFWM top frame in frame coordinates.  Corners and the
	 * plain TITLE_3 strip provide continuous top/bottom bevels with no seam. */
	os9_draw_atlas(cairo, 153, 1, 20, 20, oli, oli,
		MIN(tlw, width) - oli, height - oli);
	if (width > trw) {
		os9_draw_atlas(cairo, 174, 1, 35, 20, width - trw, oli,
			trw - oli, height - oli);
	}
	if (title_area_w > 0) {
		os9_tile_atlas_scaled_x(cairo, 128, 1, 7, 20,
			title_area_x, oli, title_area_w, height - oli);
	}

	/* Title text/window: default to a centered 80% titlebar-width plate until
	 * ssd_update_title_positions() supplies the measured font buffer. */
	int max_title_w = MAX(1, client_w * 4 / 5);
	int title_buf_x = title_x >= 0 ? title_x + left : (width - max_title_w) / 2;
	int title_buf_w = title_w > 0 ? MIN(title_w, max_title_w) : max_title_w;
	int title_start = MAX(title_area_x, title_buf_x);
	int title_end = MIN(title_area_x + title_area_w, title_start + title_buf_w);

	/* Rules: stripe band vertically centered; gaps to widgets/title equal half
	 * a widget square.  Draw only the striped source band, not the full tile. */
	int widget = rc.theme->window_button_width;
	int spacing = rc.theme->window_button_spacing;
	int half_widget = MAX(1, widget / 2);
	int left_buttons = wl_list_length(&rc.title_buttons_left);
	int right_buttons = wl_list_length(&rc.title_buttons_right);
	int left_controls_end = left;
	if (left_buttons > 0) {
		left_controls_end += left_buttons * widget + (left_buttons - 1) * spacing;
	}
	int right_controls_start = left + client_w;
	if (right_buttons > 0) {
		right_controls_start -= right_buttons * widget + (right_buttons - 1) * spacing;
	}
	int stripe_src_y = 4;
	int stripe_src_h = 12;
	int stripe_h = MIN(height, os9_tile_metric(stripe_src_h));
	int stripe_y = MAX(0, (height - stripe_h) / 2);
	int left_stripe_x = MAX(title_area_x, left_controls_end + half_widget);
	int left_stripe_w = MAX(0, title_start - half_widget - left_stripe_x);
	if (left_stripe_w > 0) {
		os9_fill_resampled_tile_x(cairo, 112, stripe_src_y, 7, stripe_src_h,
			left_stripe_x, stripe_y, left_stripe_w, stripe_h);
	}
	int right_stripe_x = MAX(title_end + half_widget, title_area_x);
	int right_stripe_end = MIN(title_area_x + title_area_w,
		right_controls_start - half_widget);
	if (right_stripe_end > right_stripe_x) {
		os9_fill_resampled_tile_x(cairo, 144, stripe_src_y, 7, stripe_src_h,
			right_stripe_x, stripe_y, right_stripe_end - right_stripe_x, stripe_h);
	}
	if (title_end > title_start) {
		/* Slightly darker neutral title backing, matching TITLE_3 rather than the
		 * very bright strip highlights, so the smaller Charcoal text sits on a
		 * classic Platinum plate. */
		os9_tile_atlas_scaled_x(cairo, 128, 1, 7, 20,
			title_start, oli, title_end - title_start, height - oli);
	}

	double line = os9_rendered_pixel();
	double L = left;
	/*
	 * Only the window's wide left/right side borders get the stylized bevel so
	 * the chrome is continuous from the titlebar into the frame. The top is kept
	 * thin (the atlas tile supplies a 1px highlight then straight into the
	 * pinstripes) so the bevel greys never intrude as an inset frame over the
	 * titlebar interior. Left border highlights the outer/left edge; right border
	 * the inner edge.
	 */
	/* Left border: highlight (white) along the outer/left edge, fading to face.
	 * This is the lit side of a raised frame. */
	cairo_rectangle(cairo, line, line, L - line, height - line);
	os9_titlebar_fill_bevel(cairo, line, 0, L, 0);
	/* Right border: the shadow side of a raised frame. Flat face plus a grey
	 * shadow line along the outer/right edge (just inside the black outline).
	 * No inner highlight, so nothing intrudes into the titlebar interior. */
	set_cairo_rgb(cairo, 0xcecece);
	cairo_rectangle(cairo, width - L, line, L - line, height - line);
	cairo_fill(cairo);
	{
		double sl = 2.0 * line;
		set_cairo_rgb(cairo, 0x9c9c9c);
		cairo_rectangle(cairo, width - line - sl, line, sl, height - line);
		cairo_fill(cairo);
	}
	/*
	 * Top edge highlight: a thin white band across the whole width, drawn AFTER
	 * the side bevels so their grey cannot overwrite it. This keeps the top
	 * highlight continuous and mitres cleanly with the left/right highlights at
	 * both top corners (a white "L"). Kept to the same 2px as the tile face
	 * highlight so it reads as the edge, not an inset frame.
	 */
	{
		double hl = 2.0 * line;
		set_cairo_rgb(cairo, 0xffffff);
		cairo_rectangle(cairo, line, line, width - 2 * line, hl);
		cairo_fill(cairo);
	}
	/* Single rendered-pixel black outline (top and both sides; no bottom line so
	 * the left/right chrome stays continuous into the client area). */
	set_cairo_rgb(cairo, 0x000000);
	cairo_rectangle(cairo, 0, 0, width, line);
	cairo_rectangle(cairo, 0, 0, line, height);
	cairo_rectangle(cairo, width - line, 0, line, height);
	cairo_fill(cairo);
	return true;
}

static void
os9_draw_outer_side_palette(cairo_t *cairo, int width, int height)
{
	static const uint32_t left_cols[] = {
		0xa6a6a6, 0x5c5c5c, 0x000000, 0x4d4d4d,
		0xe9ece9, 0xffffff, 0xe5e8e5, 0xe3e6e3,
		0xe4e7e4, 0xd7d7d7, 0xbababa, 0x8e8e8e,
	};
	static const uint32_t right_cols[] = {
		0x808080, 0xf8f8f8, 0xe4e4e4, 0xe4e4e4, 0xe4e4e4,
		0xe4e4e4, 0xd8d8d8, 0xb8b8b8, 0x5c5c5c, 0x030303, 0x303030,
	};
	int left_n = MIN((int)(sizeof(left_cols) / sizeof(left_cols[0])), width);
	for (int x = 0; x < left_n; ++x) {
		set_cairo_rgb(cairo, left_cols[x]);
		cairo_rectangle(cairo, x, 0, 1, height);
		cairo_fill(cairo);
	}
	int right_n = MIN((int)(sizeof(right_cols) / sizeof(right_cols[0])), width);
	for (int i = 0; i < right_n; ++i) {
		set_cairo_rgb(cairo, right_cols[i]);
		cairo_rectangle(cairo, width - right_n + i, 0, 1, height);
		cairo_fill(cairo);
	}
}

static void
os9_render_titlebar(cairo_t *cairo, int width, int height, bool active,
		int title_x, int title_w)
{
	if (os9_render_titlebar_from_atlas(cairo, width, height, active, title_x, title_w)) {
		return;
	}
	uint32_t face = active ? 0xd8d8d8 : 0xd0d0d0;
	uint32_t stripe_mid = active ? 0xb4b4b4 : 0xb0b0b0;
	uint32_t stripe_dark = active ? 0x909090 : 0x9a9a9a;
	uint32_t stripe_light = active ? 0xfbfbfb : 0xe8e8e8;
	set_cairo_rgb(cairo, face);
	cairo_paint(cairo);

	/*
	 * Mac OS 9 Platinum titlebars are stripe fields, not gradients.
	 * Keep a plain centre plate under the title text and draw dense horizontal
	 * 1px dark/light rules on both sides.
	 */
	int left_buttons = wl_list_length(&rc.title_buttons_left);
	int right_buttons = wl_list_length(&rc.title_buttons_right);
	int bw = rc.theme->window_button_width;
	int bs = rc.theme->window_button_spacing;
	int pad = rc.theme->window_titlebar_padding_width;
	/* Reference Platinum stripes run close to the widgets.  The wrdp-compositor
	 * button boxes include transparent/icon padding, so don't add the full
	 * theme spacing again here. */
	int stripe_left = pad + left_buttons * (bw + bs) + 8;
	int stripe_right = width - (pad + right_buttons * (bw + bs) + 4);
	int stripe_y0 = 4;
	int ramp_start = MAX(stripe_y0, height - 12);
	int stripe_y1 = ramp_start;

	/* Stripe fields fill the titlebar between the controls; the actual title
	 * text buffer paints its own tight face-colored label patch on top. */
	for (int y = stripe_y0; y < stripe_y1; y += 4) {
		set_cairo_rgb(cairo, stripe_mid);
		cairo_rectangle(cairo, stripe_left, y,
			MAX(stripe_right - stripe_left, 0), 1);
		cairo_fill(cairo);

		set_cairo_rgb(cairo, stripe_dark);
		cairo_rectangle(cairo, stripe_left, y + 1,
			MAX(stripe_right - stripe_left, 0), 1);
		cairo_fill(cairo);

		set_cairo_rgb(cairo, face);
		cairo_rectangle(cairo, stripe_left, y + 2,
			MAX(stripe_right - stripe_left, 0), 1);
		cairo_fill(cairo);

		set_cairo_rgb(cairo, stripe_light);
		cairo_rectangle(cairo, stripe_left, y + 3,
			MAX(stripe_right - stripe_left, 0), 1);
		cairo_fill(cairo);
	}

	/* Reference Desktop title plate begins slightly above the glyph buffer;
	 * draw a Platinum face underlay in the title area before the font buffer
	 * is composited.  This is deliberately narrow and centered with the same
	 * Platinum offset used for the text node. */
	if (active) {
		int plate_w = 164;
		int plate_x = (width - plate_w) / 2 - 13;
		set_cairo_rgb(cairo, 0xe9ece9);
		cairo_rectangle(cairo, plate_x, 4, plate_w, 34);
		cairo_fill(cairo);
	}

	/* Bottom metallic ramp before the client/content black field. */
	static const uint32_t bottom_ramp[] = {
		0xbfbfbf, 0xe8e8e8, 0xe8e8e8, 0xe3e3e3,
		0xe3e3e3, 0xe3e3e3, 0xe8e8e8, 0xe2e2e2,
		0xd0d0d0, 0xa1a1a1, 0x3e3e3e,
	};
	for (size_t i = 0; i < sizeof(bottom_ramp) / sizeof(bottom_ramp[0]); ++i) {
		int y = ramp_start + (int)i;
		if (y >= height - 1) {
			break;
		}
		set_cairo_rgb(cairo, bottom_ramp[i]);
		cairo_rectangle(cairo, 0, y, width, 1);
		cairo_fill(cairo);
	}

	/* Reference titlebar lead-in before the stripe field.  Avoid a hard
	 * black horizontal seam here; the black outer outline is provided by the
	 * top SSD border above. */
	set_cairo_rgb(cairo, 0xe1e1e1);
	cairo_rectangle(cairo, 0, 0, width, 1);
	cairo_fill(cairo);
	set_cairo_rgb(cairo, 0xe6e6e6);
	cairo_rectangle(cairo, 0, 1, width, 1);
	cairo_fill(cairo);
	set_cairo_rgb(cairo, 0xfdfdfd);
	cairo_rectangle(cairo, 0, 2, width, 1);
	cairo_fill(cairo);
	set_cairo_rgb(cairo, 0xf8f8f8);
	cairo_rectangle(cairo, 0, 3, width, 1);
	cairo_fill(cairo);

	if (active) {
		/* The Desktop reference has the centered title plate visible through
		 * the upper lead-in rows. Keep it centered and code-drawn, without
		 * adding side-border-looking insets. */
		int plate_w = 166;
		int plate_x = (width - plate_w) / 2 - 16;
		set_cairo_rgb(cairo, 0xe3e5e3);
		cairo_rectangle(cairo, plate_x, 2, plate_w, 2);
		cairo_fill(cairo);
	}

	/* Keep the strong square side/bottom outline and lower shadow. */
	set_cairo_rgb(cairo, 0x000000);
	cairo_rectangle(cairo, 0, 0, 1, height);
	cairo_rectangle(cairo, 0, height - 1, width, 1);
	cairo_rectangle(cairo, width - 1, 0, 1, height);
	cairo_fill(cairo);

	set_cairo_rgb(cairo, 0xffffff);
	cairo_rectangle(cairo, 1, 1, 1, height - 2);
	cairo_fill(cairo);

	set_cairo_rgb(cairo, 0x3d3d3d);
	cairo_rectangle(cairo, 1, height - 2, width - 2, 1);
	cairo_fill(cairo);
	set_cairo_rgb(cairo, 0x6f6f6f);
	cairo_rectangle(cairo, width - 2, 1, 1, height - 2);
	cairo_fill(cairo);
	if (active) {
		os9_draw_outer_side_palette(cairo, width, height);
	}
}


static struct lab_data_buffer *
os9_titlebar_buffer_create(int width, int height, bool active, float scale,
		int title_x, int title_w)
{
	width = MAX(width, 1);
	height = MAX(height, 1);

	struct lab_data_buffer *buffer = buffer_create_cairo(width, height, scale);
	if (!buffer) {
		return NULL;
	}

	cairo_t *cairo = cairo_create(buffer->surface);
	os9_render_titlebar(cairo, width, height, active, title_x, title_w);
	cairo_surface_flush(buffer->surface);
	cairo_destroy(cairo);
	return buffer;
}

static struct lab_data_buffer *
os9_titlebar_corner_buffer_create(bool right, int width, int height, float scale)
{
	struct lab_data_buffer *buffer = buffer_create_cairo(width, height, scale);
	if (!buffer) {
		return NULL;
	}
	cairo_t *cairo = cairo_create(buffer->surface);
	cairo_set_operator(cairo, CAIRO_OPERATOR_CLEAR);
	cairo_paint(cairo);
	cairo_set_operator(cairo, CAIRO_OPERATOR_OVER);
	if (right) {
		os9_draw_atlas(cairo, 174, 0, 36, 22, 0, 0, width, height);
	} else {
		os9_draw_atlas(cairo, 152, 0, 21, 22, 0, 0, width, height);
	}
	cairo_surface_flush(buffer->surface);
	cairo_destroy(cairo);
	return buffer;
}

static struct ssd_part *
add_os9_titlebar_corner(struct wl_list *parts, enum ssd_part_type type,
		struct wlr_scene_tree *parent, bool right, int x, int y, float scale)
{
	struct ssd_part *part = add_scene_part(parts, type);
	struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_create(parent, NULL);
	part->node = &scene_buffer->node;
	wlr_scene_node_set_position(part->node, x, y);
	int width = right ? os9_tile_metric(36) : os9_tile_metric(21);
	int height = os9_tile_metric(22);
	struct lab_data_buffer *buffer = os9_titlebar_corner_buffer_create(right, width, height, scale);
	if (buffer) {
		wlr_scene_buffer_set_buffer(scene_buffer, &buffer->base);
		wlr_scene_buffer_set_dest_size(scene_buffer, width, height);
		wlr_buffer_drop(&buffer->base);
	}
	return part;
}

static struct ssd_part *
add_titlebar_background(struct wl_list *parts, struct wlr_scene_tree *parent,
		int width, int height, int x, float color[4], bool active, float scale)
{
	if (!os9_platinum_theme_enabled()) {
		return add_scene_rect(parts, LAB_SSD_PART_TITLEBAR, parent,
			width, height, x, 0, color);
	}

	struct ssd_part *part = add_scene_part(parts, LAB_SSD_PART_TITLEBAR);
	struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_create(parent, NULL);
	part->node = &scene_buffer->node;
	wlr_scene_node_set_position(part->node, x, 0);

	struct lab_data_buffer *buffer = os9_titlebar_buffer_create(width, height, active, scale, -1, 0);
	if (buffer) {
		wlr_scene_buffer_set_buffer(scene_buffer, &buffer->base);
		wlr_scene_buffer_set_dest_size(scene_buffer, width, height);
		wlr_buffer_drop(&buffer->base);
	}
	return part;
}

static void
update_titlebar_background_with_title(struct ssd_part *part, int width, int height,
		int x, bool active, float scale, int title_x, int title_w)
{
	if (!part || !part->node) {
		return;
	}
	width = MAX(width, 1);
	height = MAX(height, 1);
	wlr_scene_node_set_position(part->node, x, 0);

	if (part->node->type == WLR_SCENE_NODE_RECT) {
		wlr_scene_rect_set_size(wlr_scene_rect_from_node(part->node), width, height);
		return;
	}
	if (part->node->type != WLR_SCENE_NODE_BUFFER) {
		return;
	}

	struct lab_data_buffer *buffer = os9_titlebar_buffer_create(width, height, active, scale, title_x, title_w);
	if (!buffer) {
		return;
	}
	struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_from_node(part->node);
	wlr_scene_buffer_set_buffer(scene_buffer, &buffer->base);
	wlr_scene_buffer_set_dest_size(scene_buffer, width, height);
	wlr_buffer_drop(&buffer->base);
}

static void
update_titlebar_background(struct ssd_part *part, int width, int height,
		int x, bool active, float scale)
{
	update_titlebar_background_with_title(part, width, height, x, active, scale, -1, 0);
}


static char *
os9_title_label_text(const char *title)
{
	if (!os9_platinum_theme_enabled()) {
		return xstrdup(title);
	}

	/* Pango counts leading/trailing spaces in layout width, giving the
	 * Platinum title patch its characteristic horizontal padding while the
	 * spaces remain visually blank. */
	return strdup_printf(" %s", title);
}

void
ssd_titlebar_create(struct ssd *ssd)
{
	struct view *view = ssd->view;
	struct theme *theme = view->server->theme;
	int width = view->current.width;
	int corner_width = ssd_get_corner_width();

	float *color;
	struct wlr_scene_tree *parent;
	struct wlr_buffer *corner_top_left;
	struct wlr_buffer *corner_top_right;
	int active;

	ssd->titlebar.tree = wlr_scene_tree_create(ssd->tree);

	struct ssd_sub_tree *subtree;
	FOR_EACH_STATE(ssd, subtree) {
		subtree->tree = wlr_scene_tree_create(ssd->titlebar.tree);
		parent = subtree->tree;
		active = (subtree == &ssd->titlebar.active) ?
			THEME_ACTIVE : THEME_INACTIVE;
		color = theme->window[active].title_bg_color;
		corner_top_left = &theme->window[active].corner_top_left_normal->base;
		corner_top_right = &theme->window[active].corner_top_right_normal->base;
		wlr_scene_node_set_enabled(&parent->node, active);
		wlr_scene_node_set_position(&parent->node, 0, -theme->titlebar_height);
		wl_list_init(&subtree->parts);

		/* Background */
		if (os9_platinum_theme_enabled()) {
			int left = rc.theme->border_width;
			int frame_w = width + 2 * left;
			struct ssd_part *lc = add_os9_titlebar_corner(&subtree->parts,
				LAB_SSD_PART_TITLEBAR_CORNER_LEFT, parent, false,
				-left, 0, os9_view_scale(view));
			struct ssd_part *rcp = add_os9_titlebar_corner(&subtree->parts,
				LAB_SSD_PART_TITLEBAR_CORNER_RIGHT, parent, true,
				width, 0, os9_view_scale(view));
			wlr_scene_node_set_enabled(lc->node, false);
			wlr_scene_node_set_enabled(rcp->node, false);
			add_titlebar_background(&subtree->parts, parent,
				frame_w, theme->titlebar_height, -left,
				color, active == THEME_ACTIVE, os9_view_scale(view));
		} else {
			add_titlebar_background(&subtree->parts, parent,
				width - corner_width * 2, theme->titlebar_height,
				corner_width, color, active == THEME_ACTIVE,
				os9_view_scale(view));
			add_scene_buffer(&subtree->parts, LAB_SSD_PART_TITLEBAR_CORNER_LEFT, parent,
				corner_top_left, -rc.theme->border_width, -rc.theme->border_width);
			add_scene_buffer(&subtree->parts, LAB_SSD_PART_TITLEBAR_CORNER_RIGHT, parent,
				corner_top_right, width - corner_width,
				-rc.theme->border_width);
		}

		/* Buttons */
		struct title_button *b;
		int x = theme->window_titlebar_padding_width;

		/* Center vertically within titlebar */
		int y = (theme->titlebar_height - theme->window_button_height + 1) / 2;

		wl_list_for_each(b, &rc.title_buttons_left, link) {
			struct lab_img **imgs =
				theme->window[active].button_imgs[b->type];
			add_scene_button(&subtree->parts, b->type, parent,
				imgs, x, y, view);
			x += theme->window_button_width + theme->window_button_spacing;
		}

		/* XFWM right-button x is in frame coordinates. wrdp-compositor button nodes are
		 * positioned relative to the client/titlebar origin, so subtract the
		 * left frame extent by not adding border_width here. */
		/* Nudge the right-side controls one pixel left so their outer edge does
		 * not sit on the same seam as the scaled top-right corner tile. */
		x = width + theme->window_button_spacing - 1;
		wl_list_for_each_reverse(b, &rc.title_buttons_right, link) {
			x -= theme->window_button_width + theme->window_button_spacing;
			struct lab_img **imgs =
				theme->window[active].button_imgs[b->type];
			add_scene_button(&subtree->parts, b->type, parent,
				imgs, x, y, view);
		}
	} FOR_EACH_END

	update_visible_buttons(ssd);

	ssd_update_title(ssd);
	ssd_update_window_icon(ssd);

	bool maximized = view->maximized == VIEW_AXIS_BOTH;
	bool squared = ssd_should_be_squared(ssd);
	if (maximized) {
		set_alt_button_icon(ssd, LAB_SSD_BUTTON_MAXIMIZE, true);
		ssd->state.was_maximized = true;
	}
	if (squared) {
		ssd->state.was_squared = true;
	}
	set_squared_corners(ssd, maximized || squared);

	if (view->shaded) {
		set_alt_button_icon(ssd, LAB_SSD_BUTTON_SHADE, true);
	}

	if (view->visible_on_all_workspaces) {
		set_alt_button_icon(ssd, LAB_SSD_BUTTON_OMNIPRESENT, true);
	}
}

static void
update_button_state(struct ssd_button *button, enum lab_button_state state,
		bool enable)
{
	if (enable) {
		button->state_set |= state;
	} else {
		button->state_set &= ~state;
	}
	/* Switch the displayed icon buffer to the new one */
	for (uint8_t state_set = LAB_BS_DEFAULT;
			state_set <= LAB_BS_ALL; state_set++) {
		struct scaled_img_buffer *buffer = button->img_buffers[state_set];
		if (!buffer) {
			continue;
		}
		wlr_scene_node_set_enabled(&buffer->scene_buffer->node,
			state_set == button->state_set);
	}
}

static void
set_squared_corners(struct ssd *ssd, bool enable)
{
	struct view *view = ssd->view;
	int width = view->current.width;
	int corner_width = ssd_get_corner_width();
	struct theme *theme = view->server->theme;

	struct ssd_part *part;
	struct ssd_sub_tree *subtree;
	int x = enable ? 0 : corner_width;

	FOR_EACH_STATE(ssd, subtree) {
		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLEBAR);
		if (os9_platinum_theme_enabled()) {
			int left = rc.theme->border_width;
			int frame_w = width + 2 * left;
			update_titlebar_background(part, frame_w, theme->titlebar_height,
				-left, subtree == &ssd->titlebar.active, os9_view_scale(view));
		} else {
			update_titlebar_background(part, width - 2 * x, theme->titlebar_height,
				x, subtree == &ssd->titlebar.active, os9_view_scale(view));
		}

		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLEBAR_CORNER_LEFT);
		if (part && part->node) {
			wlr_scene_node_set_enabled(part->node,
				os9_platinum_theme_enabled() ? false : !enable);
		}

		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLEBAR_CORNER_RIGHT);
		if (part && part->node) {
			wlr_scene_node_set_enabled(part->node,
				os9_platinum_theme_enabled() ? false : !enable);
		}

		/* (Un)round the corner buttons */
		struct title_button *title_button;
		wl_list_for_each(title_button, &rc.title_buttons_left, link) {
			part = ssd_get_part(&subtree->parts, title_button->type);
			struct ssd_button *button = node_ssd_button_from_node(part->node);
			update_button_state(button, LAB_BS_ROUNDED, !enable);
			break;
		}
		wl_list_for_each_reverse(title_button, &rc.title_buttons_right, link) {
			part = ssd_get_part(&subtree->parts, title_button->type);
			struct ssd_button *button = node_ssd_button_from_node(part->node);
			update_button_state(button, LAB_BS_ROUNDED, !enable);
			break;
		}
	} FOR_EACH_END
}

static void
set_alt_button_icon(struct ssd *ssd, enum ssd_part_type type, bool enable)
{
	struct ssd_part *part;
	struct ssd_button *button;
	struct ssd_sub_tree *subtree;

	FOR_EACH_STATE(ssd, subtree) {
		part = ssd_get_part(&subtree->parts, type);
		if (!part) {
			return;
		}

		button = node_ssd_button_from_node(part->node);
		update_button_state(button, LAB_BS_TOGGLED, enable);
	} FOR_EACH_END
}

/*
 * Usually this function just enables all the nodes for buttons, but some
 * buttons can be hidden for small windows (e.g. xterm -geometry 1x1).
 */
static void
update_visible_buttons(struct ssd *ssd)
{
	struct view *view = ssd->view;
	int width = view->current.width - (2 * view->server->theme->window_titlebar_padding_width);
	int button_width = view->server->theme->window_button_width;
	int button_spacing = view->server->theme->window_button_spacing;
	int button_count_left = wl_list_length(&rc.title_buttons_left);
	int button_count_right = wl_list_length(&rc.title_buttons_right);

	/* Make sure infinite loop never occurs */
	assert(button_width > 0);

	/*
	 * The corner-left button is lastly removed as it's usually a window
	 * menu button (or an app icon button in the future).
	 *
	 * There is spacing to the inside of each button, including between the
	 * innermost buttons and the window title. See also get_title_offsets().
	 */
	while (width < ((button_width + button_spacing)
			* (button_count_left + button_count_right))) {
		if (button_count_left > button_count_right) {
			button_count_left--;
		} else {
			button_count_right--;
		}
	}

	int button_count;
	struct ssd_part *part;
	struct ssd_sub_tree *subtree;
	struct title_button *b;
	FOR_EACH_STATE(ssd, subtree) {
		button_count = 0;
		wl_list_for_each(b, &rc.title_buttons_left, link) {
			part = ssd_get_part(&subtree->parts, b->type);
			wlr_scene_node_set_enabled(part->node,
				button_count < button_count_left);
			button_count++;
		}

		button_count = 0;
		wl_list_for_each_reverse(b, &rc.title_buttons_right, link) {
			part = ssd_get_part(&subtree->parts, b->type);
			wlr_scene_node_set_enabled(part->node,
				button_count < button_count_right);
			button_count++;
		}
	} FOR_EACH_END
}

void
ssd_titlebar_update(struct ssd *ssd)
{
	struct view *view = ssd->view;
	int width = view->current.width;
	int corner_width = ssd_get_corner_width();
	struct theme *theme = view->server->theme;

	bool maximized = view->maximized == VIEW_AXIS_BOTH;
	bool squared = ssd_should_be_squared(ssd);

	if (ssd->state.was_maximized != maximized
			|| ssd->state.was_squared != squared) {
		set_squared_corners(ssd, maximized || squared);
		if (ssd->state.was_maximized != maximized) {
			set_alt_button_icon(ssd, LAB_SSD_BUTTON_MAXIMIZE, maximized);
		}
		ssd->state.was_maximized = maximized;
		ssd->state.was_squared = squared;
	}

	if (ssd->state.was_shaded != view->shaded) {
		set_alt_button_icon(ssd, LAB_SSD_BUTTON_SHADE, view->shaded);
		ssd->state.was_shaded = view->shaded;
	}

	if (ssd->state.was_omnipresent != view->visible_on_all_workspaces) {
		set_alt_button_icon(ssd, LAB_SSD_BUTTON_OMNIPRESENT,
			view->visible_on_all_workspaces);
		ssd->state.was_omnipresent = view->visible_on_all_workspaces;
	}

	if (width == ssd->state.geometry.width) {
		return;
	}

	update_visible_buttons(ssd);

	/* Center buttons vertically within titlebar */
	int y = (theme->titlebar_height - theme->window_button_height + 1) / 2;
	int x;
	struct ssd_part *part;
	struct ssd_sub_tree *subtree;
	struct title_button *b;
	int bg_offset = (maximized || squared) ? 0 : corner_width;
	FOR_EACH_STATE(ssd, subtree) {
		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLEBAR);
		if (os9_platinum_theme_enabled()) {
			int left = rc.theme->border_width;
			int frame_w = width + 2 * left;
			update_titlebar_background(part, frame_w, theme->titlebar_height,
				-left, subtree == &ssd->titlebar.active, os9_view_scale(view));
			struct ssd_part *corner = ssd_get_part(&subtree->parts,
				LAB_SSD_PART_TITLEBAR_CORNER_LEFT);
			if (corner && corner->node) wlr_scene_node_set_enabled(corner->node, false);
			corner = ssd_get_part(&subtree->parts,
				LAB_SSD_PART_TITLEBAR_CORNER_RIGHT);
			if (corner && corner->node) wlr_scene_node_set_enabled(corner->node, false);
		} else {
			update_titlebar_background(part, width - bg_offset * 2,
				theme->titlebar_height, bg_offset,
				subtree == &ssd->titlebar.active, os9_view_scale(view));
		}

		x = theme->window_titlebar_padding_width;
		wl_list_for_each(b, &rc.title_buttons_left, link) {
			part = ssd_get_part(&subtree->parts, b->type);
			wlr_scene_node_set_position(part->node, x, y);
			x += theme->window_button_width + theme->window_button_spacing;
		}

		x = width - corner_width;
		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLEBAR_CORNER_RIGHT);
		wlr_scene_node_set_position(part->node, x, -rc.theme->border_width);

		x = width + theme->window_button_spacing - 1;
		wl_list_for_each_reverse(b, &rc.title_buttons_right, link) {
			part = ssd_get_part(&subtree->parts, b->type);
			x -= theme->window_button_width + theme->window_button_spacing;
			wlr_scene_node_set_position(part->node, x, y);
		}
	} FOR_EACH_END

	ssd_update_title(ssd);
	ssd_update_window_icon(ssd);
}

void
ssd_titlebar_destroy(struct ssd *ssd)
{
	if (!ssd->titlebar.tree) {
		return;
	}

	struct ssd_sub_tree *subtree;
	FOR_EACH_STATE(ssd, subtree) {
		ssd_destroy_parts(&subtree->parts);
		wlr_scene_node_destroy(&subtree->tree->node);
		subtree->tree = NULL;
	} FOR_EACH_END

	if (ssd->state.title.text) {
		zfree(ssd->state.title.text);
	}
	if (ssd->state.app_id) {
		zfree(ssd->state.app_id);
	}

	wlr_scene_node_destroy(&ssd->titlebar.tree->node);
	ssd->titlebar.tree = NULL;
}

/*
 * For ssd_update_title* we do not early out because
 * .active and .inactive may result in different sizes
 * of the title (font family/size) or background of
 * the title (different button/border width).
 *
 * Both, wlr_scene_node_set_enabled() and wlr_scene_node_set_position()
 * check for actual changes and return early if there is no change in state.
 * Always using wlr_scene_node_set_enabled(node, true) will thus not cause
 * any unnecessary screen damage and makes the code easier to follow.
 */

static void
ssd_update_title_positions(struct ssd *ssd, int offset_left, int offset_right)
{
	struct view *view = ssd->view;
	struct theme *theme = view->server->theme;
	int width = view->current.width;
	int title_bg_width = width - offset_left - offset_right;

	int x, y;
	int buffer_height, buffer_width;
	struct ssd_part *part;
	struct ssd_sub_tree *subtree;
	FOR_EACH_STATE(ssd, subtree) {
		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLE);
		if (!part || !part->node) {
			/* view->surface never been mapped */
			/* Or we somehow failed to allocate a scaled titlebar buffer */
			continue;
		}

		buffer_width = part->buffer ? part->buffer->width : 0;
		buffer_height = part->buffer ? part->buffer->height : 0;
		x = offset_left;
		y = (theme->titlebar_height - buffer_height) / 2;
		if (os9_platinum_theme_enabled()) {
			/* Render the title one scaled pixel lower over the transparent
			 * titlebar background. */
			y += os9_line_metric();
		}

		if (title_bg_width <= 0) {
			wlr_scene_node_set_enabled(part->node, false);
			continue;
		}
		wlr_scene_node_set_enabled(part->node, true);

		if (theme->window_label_text_justify == LAB_JUSTIFY_CENTER) {
			if (buffer_width + MAX(offset_left, offset_right) * 2 <= width) {
				/* Center based on the full width */
				x = (width - buffer_width) / 2;
				if (os9_platinum_theme_enabled()) {
					/* The reference Desktop title sits left of geometric center.
					 * With Platinum Charcoal AA-none and 192 letter spacing, -16px
					 * best matches the canonical title bbox in the Desktop reference. */
					x -= 16;
				}
			} else {
				/*
				 * Center based on the width between the buttons.
				 * Title jumps around once this is hit but its still
				 * better than to hide behind the buttons on the right.
				 */
				x += (title_bg_width - buffer_width) / 2;
			}
		} else if (theme->window_label_text_justify == LAB_JUSTIFY_RIGHT) {
			x += title_bg_width - buffer_width;
		} else if (theme->window_label_text_justify == LAB_JUSTIFY_LEFT) {
			/* TODO: maybe add some theme x padding here? */
		}
		wlr_scene_node_set_position(part->node, x, y);
		if (os9_platinum_theme_enabled()) {
			struct ssd_part *bg = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLEBAR);
			int left = rc.theme->border_width;
			update_titlebar_background_with_title(bg, width + 2 * left,
				theme->titlebar_height, -left, subtree == &ssd->titlebar.active,
				os9_view_scale(view), x, buffer_width);
		}
	} FOR_EACH_END
}

/*
 * Get left/right offsets of the title area based on visible/hidden states of
 * buttons set in update_visible_buttons().
 */
static void
get_title_offsets(struct ssd *ssd, int *offset_left, int *offset_right)
{
	struct ssd_sub_tree *subtree = &ssd->titlebar.active;
	int button_width = ssd->view->server->theme->window_button_width;
	int button_spacing = ssd->view->server->theme->window_button_spacing;
	int padding_width = ssd->view->server->theme->window_titlebar_padding_width;
	*offset_left = padding_width;
	*offset_right = padding_width;

	struct title_button *b;
	wl_list_for_each(b, &rc.title_buttons_left, link) {
		struct ssd_part *part = ssd_get_part(&subtree->parts, b->type);
		if (part->node->enabled) {
			*offset_left += button_width + button_spacing;
		}
	}
	wl_list_for_each_reverse(b, &rc.title_buttons_right, link) {
		struct ssd_part *part = ssd_get_part(&subtree->parts, b->type);
		if (part->node->enabled) {
			*offset_right += button_width + button_spacing;
		}
	}
}

void
ssd_update_title(struct ssd *ssd)
{
	if (!ssd || !rc.show_title) {
		return;
	}

	struct view *view = ssd->view;
	char *title = (char *)view_get_string_prop(view, "title");
	if (string_null_or_empty(title)) {
		return;
	}

	struct theme *theme = view->server->theme;
	struct ssd_state_title *state = &ssd->state.title;
	bool title_unchanged = state->text && !strcmp(title, state->text);

	const float *text_color;
	const float *bg_color;
	struct font *font = NULL;
	struct ssd_part *part;
	struct ssd_sub_tree *subtree;
	struct ssd_state_title_width *dstate;
	int active;

	int offset_left, offset_right;
	get_title_offsets(ssd, &offset_left, &offset_right);
	int title_bg_width = view->current.width - offset_left - offset_right;
	if (os9_platinum_theme_enabled()) {
		title_bg_width = MIN(title_bg_width, view->current.width * 4 / 5);
	}

	FOR_EACH_STATE(ssd, subtree) {
		active = (subtree == &ssd->titlebar.active) ?
			THEME_ACTIVE : THEME_INACTIVE;
		dstate = active ? &state->active : &state->inactive;
		text_color = theme->window[active].label_text_color;
		bg_color = theme->window[active].title_bg_color;
		static const float os9_transparent_title_bg[4] = {0.0f, 0.0f, 0.0f, 0.0f};
		if (os9_platinum_theme_enabled()) {
			/* Render glyph alpha directly over the already-rendered titlebar;
			 * do not paint an opaque backing rectangle behind the title text. */
			bg_color = os9_transparent_title_bg;
		}
		font = active ?  &rc.font_activewindow : &rc.font_inactivewindow;

		if (title_bg_width <= 0) {
			dstate->truncated = true;
			continue;
		}

		if (title_unchanged
				&& !dstate->truncated && dstate->width < title_bg_width) {
			/* title the same + we don't need to resize title */
			continue;
		}

		part = ssd_get_part(&subtree->parts, LAB_SSD_PART_TITLE);
		if (!part) {
			/* Initialize part and wlr_scene_buffer without attaching a buffer */
			part = add_scene_part(&subtree->parts, LAB_SSD_PART_TITLE);
			part->buffer = scaled_font_buffer_create(subtree->tree);
			if (part->buffer) {
				part->node = &part->buffer->scene_buffer->node;
			} else {
				wlr_log(WLR_ERROR, "Failed to create title node");
			}
		}

		if (part->buffer) {
			char *label_text = os9_title_label_text(title);
			scaled_font_buffer_update(part->buffer, label_text,
				title_bg_width, font,
				text_color, bg_color);
			free(label_text);
		}

		/* And finally update the cache */
		dstate->width = part->buffer ? part->buffer->width : 0;
		dstate->truncated = title_bg_width <= dstate->width;

	} FOR_EACH_END

	if (!title_unchanged) {
		if (state->text) {
			free(state->text);
		}
		state->text = xstrdup(title);
	}
	ssd_update_title_positions(ssd, offset_left, offset_right);
}

void
ssd_update_button_hover(struct wlr_scene_node *node,
		struct ssd_hover_state *hover_state)
{
	struct ssd_button *button = NULL;
	if (!node || !node->data) {
		goto disable_old_hover;
	}

	struct node_descriptor *desc = node->data;
	if (desc->type == LAB_NODE_DESC_SSD_BUTTON) {
		button = node_ssd_button_from_node(node);
		if (button == hover_state->button) {
			/* Cursor is still on the same button */
			return;
		}
	}

disable_old_hover:
	if (hover_state->button) {
		update_button_state(hover_state->button, LAB_BS_HOVERD, false);
		hover_state->view = NULL;
		hover_state->button = NULL;
	}
	if (button) {
		update_button_state(button, LAB_BS_HOVERD, true);
		hover_state->view = button->view;
		hover_state->button = button;
	}
}

bool
ssd_should_be_squared(struct ssd *ssd)
{
	struct view *view = ssd->view;
	int corner_width = ssd_get_corner_width();

	return (view_is_tiled_and_notify_tiled(view)
			|| view->current.width < corner_width * 2)
		&& view->maximized != VIEW_AXIS_BOTH;
}

void
ssd_update_window_icon(struct ssd *ssd)
{
#if HAVE_LIBSFDO
	if (!ssd) {
		return;
	}

	const char *app_id = view_get_string_prop(ssd->view, "app_id");
	if (string_null_or_empty(app_id)) {
		return;
	}
	if (ssd->state.app_id && !strcmp(ssd->state.app_id, app_id)) {
		return;
	}

	free(ssd->state.app_id);
	ssd->state.app_id = xstrdup(app_id);

	struct ssd_sub_tree *subtree;
	FOR_EACH_STATE(ssd, subtree) {
		struct ssd_part *part = ssd_get_part(
			&subtree->parts, LAB_SSD_BUTTON_WINDOW_ICON);
		if (!part) {
			break;
		}

		struct ssd_button *button = node_ssd_button_from_node(part->node);
		assert(button->window_icon);
		scaled_icon_buffer_set_app_id(button->window_icon, app_id);
	} FOR_EACH_END
#endif
}

#undef FOR_EACH_STATE
