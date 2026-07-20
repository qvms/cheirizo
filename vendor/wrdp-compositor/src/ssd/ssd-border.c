// SPDX-License-Identifier: GPL-2.0-only
/* WRDP modifications, 2026. */

#include <assert.h>
#include <stdlib.h>
#include <string.h>
#include "buffer.h"
#include "common/scene-helpers.h"
#include "wrdp-compositor.h"
#include "ssd-internal.h"
#include "theme.h"
#include "view.h"

#define FOR_EACH_STATE(ssd, tmp) FOR_EACH(tmp, \
	&(ssd)->border.active, \
	&(ssd)->border.inactive)

static bool
os9_platinum_theme_enabled(void)
{
	return rc.theme_name && (strstr(rc.theme_name, "Platinum")
		|| strstr(rc.theme_name, "Mac-OS-9")
		|| strstr(rc.theme_name, "MacOS9"));
}

static float
os9_view_scale(struct view *view)
{
	if (view && view->output && view->output->wlr_output
			&& view->output->wlr_output->scale > 0) {
		return view->output->wlr_output->scale;
	}
	return 1.0f;
}

/*
 * Platinum was authored for the classic Mac 72 dpi UI model. In RDP, a
 * Windows-style 150% client effectively presents 96 * 1.5 = 144 dpi, so a
 * classic Mac pixel maps to 144 / 72 = 2 device-density pixels. Keep scene
 * geometry stable, but compress authored detail coordinates by this ratio so
 * the decorative strokes do not read as a 2x bitmap. Override with
 * WRDP_COMPOSITOR_OS9_AUTHORING_SCALE, or WRDP_COMPOSITOR_OS9_CLIENT_SCALE where 1.5 means 150%.
 */
static float
os9_authoring_scale(void)
{
	const char *authoring = getenv("WRDP_COMPOSITOR_OS9_AUTHORING_SCALE");
	if (authoring && *authoring) {
		char *end = NULL;
		float parsed = strtof(authoring, &end);
		if (end != authoring && parsed >= 1.0f && parsed <= 4.0f) {
			return parsed;
		}
	}
	float client_scale = 1.5f;
	const char *client = getenv("WRDP_COMPOSITOR_OS9_CLIENT_SCALE");
	if (client && *client) {
		char *end = NULL;
		float parsed = strtof(client, &end);
		if (end != client && parsed >= 1.0f && parsed <= 3.0f) {
			client_scale = parsed;
		}
	}
	return (96.0f * client_scale) / 72.0f;
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

static void
fill_rect_rgb(cairo_t *cairo, int x, int y, int w, int h, uint32_t rgb)
{
	if (w <= 0 || h <= 0) {
		return;
	}
	set_cairo_rgb(cairo, rgb);
	cairo_rectangle(cairo, x, y, w, h);
	cairo_fill(cairo);
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

/*
 * Set a smooth Platinum bevel gradient as the cairo source along the vector
 * (x0,y0)->(x1,y1). 0.0 is the light-facing edge (highlight), 1.0 is the
 * shadow edge. Profile derived from the taoofmac Platinum reference borders:
 * a bright highlight just inside the outline that falls off smoothly to a dark
 * inner edge. Caller owns the returned pattern and must destroy it after use.
 */
static cairo_pattern_t *
os9_bevel_pattern(double x0, double y0, double x1, double y1)
{
	cairo_pattern_t *pat = cairo_pattern_create_linear(x0, y0, x1, y1);
	/* Stylized flat bands (no smooth ramp) matching the atlas widgets and
	 * stripes: white highlight (0xff) -> light grey (0xce) -> medium grey
	 * (0x9c), from the light-facing edge to the shadow edge. */
	cairo_pattern_add_color_stop_rgb(pat, 0.00, 1.00, 1.00, 1.00);
	cairo_pattern_add_color_stop_rgb(pat, 0.26, 1.00, 1.00, 1.00);
	cairo_pattern_add_color_stop_rgb(pat, 0.2601, 0.808, 0.808, 0.808);
	cairo_pattern_add_color_stop_rgb(pat, 0.70, 0.808, 0.808, 0.808);
	cairo_pattern_add_color_stop_rgb(pat, 0.7001, 0.612, 0.612, 0.612);
	cairo_pattern_add_color_stop_rgb(pat, 1.00, 0.612, 0.612, 0.612);
	return pat;
}

static void
os9_fill_bevel(cairo_t *cairo, double bx0, double by0, double bx1, double by1)
{
	cairo_pattern_t *pat = os9_bevel_pattern(bx0, by0, bx1, by1);
	cairo_set_source(cairo, pat);
	cairo_fill(cairo);
	cairo_pattern_destroy(pat);
}

static bool
os9_render_border_from_atlas(cairo_t *cairo, int width, int height,
		enum ssd_part_type type, bool shaded)
{
	if (!os9_platinum_atlas_surface()) {
		return false;
	}
	cairo_set_operator(cairo, CAIRO_OPERATOR_CLEAR);
	cairo_paint(cairo);
	cairo_set_operator(cairo, CAIRO_OPERATOR_OVER);
	switch (type) {
	case LAB_SSD_PART_LEFT: {
		/* Left border: highlight against the outer (left) edge, fading inward. */
		double line = os9_rendered_pixel();
		cairo_rectangle(cairo, line, 0, width - line, height);
		os9_fill_bevel(cairo, line, 0, width, 0);
		set_cairo_rgb(cairo, 0x000000);
		cairo_rectangle(cairo, 0, 0, line, height);
		cairo_fill(cairo);
		return true;
	}
	case LAB_SSD_PART_RIGHT: {
		/* Right border: highlight against the inner (client) edge, fading out. */
		double line = os9_rendered_pixel();
		cairo_rectangle(cairo, 0, 0, width - line, height);
		os9_fill_bevel(cairo, 0, 0, width - line, 0);
		set_cairo_rgb(cairo, 0x000000);
		cairo_rectangle(cairo, width - line, 0, line, height);
		cairo_fill(cairo);
		return true;
	}
	case LAB_SSD_PART_TOP:
		/* XFWM top pixels belong to top corners + title window, not a separate
		 * wrdp-compositor border layer. Keep the part transparent to avoid double borders. */
		return true;
	case LAB_SSD_PART_BOTTOM:
		/* Bottom bar: highlight against the inner (top/client) edge, fading to
		 * a dark shadow at the outer (bottom) edge. The bottom-left and
		 * bottom-right corners are mitred at 45 degrees by clipping to a
		 * triangle and repainting the adjoining side's bevel so the highlight
		 * turns the corner exactly like the reference. Single rendered-pixel
		 * outline; identical for shaded and unshaded. */
		(void)shaded;
		{
			double line = os9_rendered_pixel();
			int corner = os9_tile_metric(6);
			if (corner > width / 2) {
				corner = width / 2;
			}
			/* Opaque metallic base (covers any rounding gaps). */
			set_cairo_rgb(cairo, 0xcecece);
			cairo_rectangle(cairo, 0, 0, width, height);
			cairo_fill(cairo);
			/* Vertical bottom bevel across the whole bar (highlight at top). */
			cairo_rectangle(cairo, 0, 0, width, height - line);
			os9_fill_bevel(cairo, 0, 0, 0, height - line);
			/* Bottom-left corner: clip to the upper-left triangle (bounded by
			 * the top edge, left edge and the 45-degree diagonal from the outer
			 * corner) and repaint with the left border's bevel (highlight at the
			 * outer/left edge). */
			cairo_save(cairo);
			cairo_move_to(cairo, 0, 0);
			cairo_line_to(cairo, corner, 0);
			cairo_line_to(cairo, 0, height);
			cairo_close_path(cairo);
			cairo_clip(cairo);
			cairo_new_path(cairo);
			cairo_rectangle(cairo, line, 0, corner, height - line);
			os9_fill_bevel(cairo, line, 0, corner, 0);
			cairo_restore(cairo);
			/* Bottom-right corner: upper-right triangle, right border's bevel
			 * (highlight at the inner edge). */
			cairo_save(cairo);
			cairo_move_to(cairo, width, 0);
			cairo_line_to(cairo, width - corner, 0);
			cairo_line_to(cairo, width, height);
			cairo_close_path(cairo);
			cairo_clip(cairo);
			cairo_new_path(cairo);
			cairo_rectangle(cairo, width - corner, 0, corner - line, height - line);
			os9_fill_bevel(cairo, width - corner, 0, width - line, 0);
			cairo_restore(cairo);
			/* Single rendered-pixel black outline (outer edges only). */
			set_cairo_rgb(cairo, 0x000000);
			cairo_rectangle(cairo, 0, height - line, width, line);
			cairo_rectangle(cairo, 0, 0, line, height);
			cairo_rectangle(cairo, width - line, 0, line, height);
			cairo_fill(cairo);
		}
		return true;
	default:
		return false;
	}
}

static void
os9_render_border(cairo_t *cairo, int width, int height,
		enum ssd_part_type type, float authoring_scale, bool shaded)
{
	if (os9_render_border_from_atlas(cairo, width, height, type, shaded)) {
		return;
	}
	fill_rect_rgb(cairo, 0, 0, width, height, 0xd8d8d8);

	/*
	 * Platinum metal edge: black contrast outline, then white highlight,
	 * mid grey face and dark shadow. Which side gets the black line depends on
	 * which edge is physically outside the window.
	 */
	switch (type) {
	case LAB_SSD_PART_LEFT:
		if (width >= 16) {
			/* Wide Platinum body bevel: an outer retained layout edge,
			 * then an inner dark cut-line, white highlight, soft metal face,
			 * and final shadow before the client. */
			fill_rect_rgb(cairo, 0, 0, width, height, 0xd8d8d8);
			fill_rect_rgb(cairo, 3, 0, 1, height, 0x9d998f);
			fill_rect_rgb(cairo, 4, 0, 1, height, 0x57544f);
			fill_rect_rgb(cairo, 5, 0, 1, height, 0x000000);
			fill_rect_rgb(cairo, 6, 0, 1, height, 0x4d4d4d);
			fill_rect_rgb(cairo, 7, 0, 1, height, 0xe8e8e8);
			fill_rect_rgb(cairo, 8, 0, 1, height, 0xffffff);
			fill_rect_rgb(cairo, 9, 0, 1, height, 0xe6e6e6);
			fill_rect_rgb(cairo, 10, 0, 1, height, 0xe2e2e2);
			fill_rect_rgb(cairo, 11, 0, 1, height, 0xe3e3e3);
			fill_rect_rgb(cairo, 12, 0, 1, height, 0xd8d8d8);
			fill_rect_rgb(cairo, 13, 0, 1, height, 0xbdbdbd);
			fill_rect_rgb(cairo, 14, 0, 1, height, 0x707070);
			fill_rect_rgb(cairo, 15, 0, 1, height, 0x0f0f0f);
		} else {
			fill_rect_rgb(cairo, 0, 0, 1, height, 0x000000);
			fill_rect_rgb(cairo, 1, 0, 1, height, 0xffffff);
			fill_rect_rgb(cairo, 2, 0, 1, height, 0xd8d8d8);
			fill_rect_rgb(cairo, 3, 0, 1, height, 0x959595);
			fill_rect_rgb(cairo, width - 2, 0, 1, height, 0x525252);
			fill_rect_rgb(cairo, width - 1, 0, 1, height, 0x000000);
		}
		break;
	case LAB_SSD_PART_RIGHT:
		if (width >= 16) {
			/* Mirror the wide Platinum body bevel for the right edge. */
			fill_rect_rgb(cairo, 0, 0, width, height, 0xd8d8d8);
			fill_rect_rgb(cairo, 0, 0, 1, height, 0x000000);
			fill_rect_rgb(cairo, 1, 0, 1, height, 0x000000);
			fill_rect_rgb(cairo, 2, 0, 1, height, 0x6c6c6c);
			fill_rect_rgb(cairo, 3, 0, 1, height, 0xf6f6f6);
			fill_rect_rgb(cairo, 4, 0, 1, height, 0xffffff);
			fill_rect_rgb(cairo, 5, 0, 1, height, 0xe6e6e6);
			fill_rect_rgb(cairo, 6, 0, 1, height, 0xe3e3e3);
			fill_rect_rgb(cairo, 7, 0, 1, height, 0xe3e3e3);
			fill_rect_rgb(cairo, 8, 0, 1, height, 0xd6d6d6);
			fill_rect_rgb(cairo, 9, 0, 1, height, 0xb8b8b8);
			fill_rect_rgb(cairo, 10, 0, 1, height, 0x5c5c5c);
			fill_rect_rgb(cairo, 11, 0, 1, height, 0x000000);
			fill_rect_rgb(cairo, 12, 0, 1, height, 0x302e2a);
		} else {
			fill_rect_rgb(cairo, 0, 0, 1, height, 0x5c5c5c);
			fill_rect_rgb(cairo, 1, 0, 1, height, 0x030303);
			fill_rect_rgb(cairo, 2, 0, 1, height, 0x2b2b2b);
			fill_rect_rgb(cairo, 3, 0, 1, height, 0x989898);
			fill_rect_rgb(cairo, width - 2, 0, 1, height, 0x555555);
			fill_rect_rgb(cairo, width - 1, 0, 1, height, 0x000000);
		}
		break;
	case LAB_SSD_PART_TOP:
		/* Empirically match the Desktop reference top metallic lead-in.
		 * wrdp-compositor places the top SSD part with the first rendered row clipped
		 * against the outer edge, so row order is chosen to reproduce the
		 * visible sequence: #282828, #0b0b0b, #9b9b9b, #ffffff,
		 * #fafafa, #e3e3e3. */
		if (height >= 6) {
			fill_rect_rgb(cairo, 0, 0, width, 1, 0x252422);
			fill_rect_rgb(cairo, 0, 1, width, 1, 0x0b0b0b);
			fill_rect_rgb(cairo, 0, 2, width, 1, 0x9b9b9b);
			fill_rect_rgb(cairo, 0, 3, width, 1, 0xffffff);
			fill_rect_rgb(cairo, 0, 4, width, 1, 0xfafafa);
			fill_rect_rgb(cairo, 0, 5, width, 1, 0xe3e3e3);
			/* Square Platinum corner: continue the right edge shadow through
			 * the bright top-border rows without changing widget geometry. */
			fill_rect_rgb(cairo, width - 6, 2, 6, 4, 0x505050);
			/* Match the left square corner cap to the reference shadowed edge. */
			fill_rect_rgb(cairo, 0, 2, 6, 4, 0x5a5a5a);
			if (height > 6) {
				fill_rect_rgb(cairo, 0, 6, width, height - 6, 0xe0e0e0);
			}
		} else {
			fill_rect_rgb(cairo, 0, 0, width, height, 0xd8d8d8);
		}
		break;
	case LAB_SSD_PART_BOTTOM:
		if (height >= 11) {
			/* Procedural Platinum bottom lip: a compact metallic ramp that
			 * overlaps the client while keeping the logical border width. */
			static const uint32_t bottom_lip[] = {
				0x212121, 0xb8b8b8, 0xffffff, 0xf4f4f4,
				0xe2e2e2, 0xe6e6e6, 0xdfdfdf, 0xcccccc,
				0x949494, 0x262626, 0x080808,
			};
			for (size_t i = 0; i < sizeof(bottom_lip) / sizeof(bottom_lip[0]); ++i) {
				fill_rect_rgb(cairo, 0, (int)i, width, 1, bottom_lip[i]);
			}
			/* No lower-left cap overlay: avoid the visible extra outline. */
			fill_rect_rgb(cairo, width - 10, 0, 10, 1, 0xc0c0c0);
			fill_rect_rgb(cairo, 7, 1, 3, 1, 0xe0e0e0);
			fill_rect_rgb(cairo, width - 10, 2, 10, 1, 0xc0c0c0);
			fill_rect_rgb(cairo, width - 6, 2, 1, 1, 0x606060);
			fill_rect_rgb(cairo, width - 5, 2, 1, 1, 0x202020);
			fill_rect_rgb(cairo, width - 4, 2, 1, 1, 0x303030);
			fill_rect_rgb(cairo, width - 5, 3, 5, 1, 0x707070);
			fill_rect_rgb(cairo, width - 7, 3, 1, 1, 0xb0b0b0);
			fill_rect_rgb(cairo, width - 5, 3, 1, 1, 0x202020);
			fill_rect_rgb(cairo, width - 4, 3, 1, 1, 0x303030);
			fill_rect_rgb(cairo, width - 6, 3, 1, 1, 0x606060);
			fill_rect_rgb(cairo, width - 6, 4, 1, 1, 0x606060);
			fill_rect_rgb(cairo, width - 5, 4, 1, 1, 0x202020);
			fill_rect_rgb(cairo, width - 4, 4, 1, 1, 0x303030);
			fill_rect_rgb(cairo, width - 7, 5, 1, 1, 0xb0b0b0);
			fill_rect_rgb(cairo, width - 6, 5, 3, 1, 0x303030);
			fill_rect_rgb(cairo, width - 6, 6, 1, 1, 0x606060);
			fill_rect_rgb(cairo, width - 5, 6, 1, 1, 0x202020);
			fill_rect_rgb(cairo, width - 4, 6, 1, 1, 0x303030);
			fill_rect_rgb(cairo, width - 6, 7, 1, 1, 0x606060);
			fill_rect_rgb(cairo, width - 5, 7, 1, 1, 0x202020);
			fill_rect_rgb(cairo, width - 4, 7, 1, 1, 0x303030);
		} else {
			fill_rect_rgb(cairo, 0, 0, width, 1, 0xe6e6e6);
			fill_rect_rgb(cairo, 0, 1, width, MAX(height - 5, 0), 0xe0e0e0);
			fill_rect_rgb(cairo, 0, height - 4, width, 1, 0xcccccc);
			fill_rect_rgb(cairo, 0, height - 3, width, 1, 0x949494);
			fill_rect_rgb(cairo, 0, height - 2, width, 1, 0x242424);
			fill_rect_rgb(cairo, 0, height - 1, width, 1, 0x000000);
		}
		break;
	default:
		break;
	}
}

static struct lab_data_buffer *
os9_border_buffer_create(int width, int height, enum ssd_part_type type, float scale,
		bool shaded)
{
	width = MAX(width, 1);
	height = MAX(height, 1);
	struct lab_data_buffer *buffer = buffer_create_cairo(width, height, scale);
	if (!buffer) {
		return NULL;
	}
	cairo_t *cairo = cairo_create(buffer->surface);
	os9_render_border(cairo, width, height, type, os9_authoring_scale(), shaded);
	cairo_surface_flush(buffer->surface);
	cairo_destroy(cairo);
	return buffer;
}

static struct ssd_part *
add_border_part(struct wl_list *parts, enum ssd_part_type type,
		struct wlr_scene_tree *parent, int width, int height, int x, int y,
		float color[4], float scale, bool shaded)
{
	if (!os9_platinum_theme_enabled()) {
		return add_scene_rect(parts, type, parent, width, height, x, y, color);
	}
	struct ssd_part *part = add_scene_part(parts, type);
	struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_create(parent, NULL);
	part->node = &scene_buffer->node;
	wlr_scene_node_set_position(part->node, x, y);
	struct lab_data_buffer *buffer = os9_border_buffer_create(width, height, type, scale, shaded);
	if (buffer) {
		wlr_scene_buffer_set_buffer(scene_buffer, &buffer->base);
		wlr_scene_buffer_set_dest_size(scene_buffer, width, height);
		wlr_buffer_drop(&buffer->base);
	}
	return part;
}

static void
update_border_part(struct ssd_part *part, int width, int height, int x, int y,
		float scale, bool shaded)
{
	if (!part || !part->node) {
		return;
	}
	width = MAX(width, 1);
	height = MAX(height, 1);
	wlr_scene_node_set_position(part->node, x, y);
	if (part->node->type == WLR_SCENE_NODE_RECT) {
		wlr_scene_rect_set_size(wlr_scene_rect_from_node(part->node), width, height);
		return;
	}
	if (part->node->type != WLR_SCENE_NODE_BUFFER) {
		return;
	}
	struct lab_data_buffer *buffer = os9_border_buffer_create(width, height, part->type, scale, shaded);
	if (!buffer) {
		return;
	}
	struct wlr_scene_buffer *scene_buffer = wlr_scene_buffer_from_node(part->node);
	wlr_scene_buffer_set_buffer(scene_buffer, &buffer->base);
	wlr_scene_buffer_set_dest_size(scene_buffer, width, height);
	wlr_buffer_drop(&buffer->base);
}

void
ssd_border_create(struct ssd *ssd)
{
	assert(ssd);
	assert(!ssd->border.tree);

	struct view *view = ssd->view;
	struct theme *theme = view->server->theme;
	int width = view->current.width;
	int height = view_effective_height(view, /* use_pending */ false);
	int full_width = width + 2 * theme->border_width;
	int corner_width = ssd_get_corner_width();

	float *color;
	struct wlr_scene_tree *parent;
	struct ssd_sub_tree *subtree;
	int active;

	ssd->border.tree = wlr_scene_tree_create(ssd->tree);
	wlr_scene_node_set_position(&ssd->border.tree->node, -theme->border_width, 0);
	if (os9_platinum_theme_enabled()) {
		/* A Platinum frame is painted above the client surface; wrdp-compositor's
		 * default SSD tree is below the surface because normal borders do
		 * not overlap client pixels. */
		wlr_scene_node_raise_to_top(&ssd->tree->node);
	}

	FOR_EACH_STATE(ssd, subtree) {
		subtree->tree = wlr_scene_tree_create(ssd->border.tree);
		parent = subtree->tree;
		active = (subtree == &ssd->border.active) ?
			THEME_ACTIVE : THEME_INACTIVE;
		wlr_scene_node_set_enabled(&parent->node, active);
		color = theme->window[active].border_color;

		wl_list_init(&subtree->parts);
		int side_width = theme->border_width;
		int side_inset = 0;
		add_border_part(&subtree->parts, LAB_SSD_PART_LEFT, parent,
			side_width, height, 0, 0, color, os9_view_scale(view), view->shaded);
		add_border_part(&subtree->parts, LAB_SSD_PART_RIGHT, parent,
			side_width, height,
			theme->border_width + width - side_inset, 0, color, os9_view_scale(view), view->shaded);
		int bottom_height = view->shaded ? os9_tile_metric(6) : theme->border_width;
		int bottom_y = view->shaded ? 0 : height;
		add_border_part(&subtree->parts, LAB_SSD_PART_BOTTOM, parent,
			full_width, bottom_height, 0, bottom_y, color, os9_view_scale(view), view->shaded);
		if (os9_platinum_theme_enabled()) {
			/* OS 9 Platinum uses square top corners; the metallic top border
			 * spans the full decorated width in direct code, not via corner art. */
			add_border_part(&subtree->parts, LAB_SSD_PART_TOP, parent,
				full_width, theme->border_width, 0,
				-(ssd->titlebar.height + theme->border_width), color, os9_view_scale(view), view->shaded);
		} else {
			add_border_part(&subtree->parts, LAB_SSD_PART_TOP, parent,
				width - 2 * corner_width, theme->border_width,
				theme->border_width + corner_width,
				-(ssd->titlebar.height + theme->border_width), color, os9_view_scale(view), view->shaded);
		}
	} FOR_EACH_END

	if (view->maximized == VIEW_AXIS_BOTH) {
		wlr_scene_node_set_enabled(&ssd->border.tree->node, false);
	}

	if (view->current.width > 0 && view->current.height > 0) {
		/*
		 * The SSD is recreated by a Reconfigure request
		 * thus we may need to handle squared corners.
		 */
		ssd_border_update(ssd);
	}
}

void
ssd_border_update(struct ssd *ssd)
{
	assert(ssd);
	assert(ssd->border.tree);

	struct view *view = ssd->view;
	if (os9_platinum_theme_enabled()) {
		wlr_scene_node_raise_to_top(&ssd->tree->node);
	}

	if (view->maximized == VIEW_AXIS_BOTH
			&& ssd->border.tree->node.enabled) {
		/* Disable borders on maximize */
		wlr_scene_node_set_enabled(&ssd->border.tree->node, false);
		ssd->margin = ssd_thickness(ssd->view);
	}

	if (view->maximized == VIEW_AXIS_BOTH) {
		return;
	} else if (!ssd->border.tree->node.enabled) {
		/* And re-enabled them when unmaximized */
		wlr_scene_node_set_enabled(&ssd->border.tree->node, true);
		ssd->margin = ssd_thickness(ssd->view);
	}

	struct theme *theme = view->server->theme;

	int width = view->current.width;
	int height = view_effective_height(view, /* use_pending */ false);
	int full_width = width + 2 * theme->border_width;
	int corner_width = ssd_get_corner_width();

	/*
	 * From here on we have to cover the following border scenarios:
	 * Non-tiled (partial border, rounded corners):
	 *    _____________
	 *   o           oox
	 *  |---------------|
	 *  |_______________|
	 *
	 * Tiled (full border, squared corners):
	 *   _______________
	 *  |o           oox|
	 *  |---------------|
	 *  |_______________|
	 *
	 * Tiled or non-tiled with zero title height (full boarder, no title):
	 *   _______________
	 *  |_______________|
	 */

	int side_height = view->shaded ? ssd->titlebar.height :
		(ssd->state.was_squared ? height + ssd->titlebar.height : height);
	int side_y = ssd->state.was_squared && !view->shaded
		? -ssd->titlebar.height
		: 0;
	int top_width = ssd->titlebar.height <= 0 || ssd->state.was_squared
		? full_width
		: width - 2 * corner_width;
	int top_x = ssd->titlebar.height <= 0 || ssd->state.was_squared
		? 0
		: theme->border_width + corner_width;
	if (os9_platinum_theme_enabled()) {
		top_width = full_width;
		top_x = 0;
	}

	struct ssd_part *part;
	struct ssd_sub_tree *subtree;
	FOR_EACH_STATE(ssd, subtree) {
		wl_list_for_each(part, &subtree->parts, link) {
			switch (part->type) {
			case LAB_SSD_PART_LEFT: {
				if (view->shaded) {
					wlr_scene_node_set_enabled(part->node, false);
					continue;
				}
				wlr_scene_node_set_enabled(part->node, true);
				int side_width = theme->border_width;
				update_border_part(part, side_width, side_height, 0, side_y, os9_view_scale(view), view->shaded);
				continue;
			}
			case LAB_SSD_PART_RIGHT: {
				if (view->shaded) {
					wlr_scene_node_set_enabled(part->node, false);
					continue;
				}
				wlr_scene_node_set_enabled(part->node, true);
				int side_width = theme->border_width;
				update_border_part(part, side_width, side_height,
					theme->border_width + width, side_y, os9_view_scale(view), view->shaded);
				continue;
			}
			case LAB_SSD_PART_BOTTOM: {
				wlr_scene_node_set_enabled(part->node, true);
				int bottom_height = view->shaded ? os9_tile_metric(6) : theme->border_width;
				int bottom_y = view->shaded ? 0 : height;
				update_border_part(part, full_width, bottom_height, 0, bottom_y, os9_view_scale(view), view->shaded);
				continue;
			}
			case LAB_SSD_PART_TOP:
				update_border_part(part, top_width, theme->border_width, top_x,
					-(ssd->titlebar.height + theme->border_width), os9_view_scale(view), view->shaded);
				continue;
			default:
				continue;
			}
		}
	} FOR_EACH_END
}

void
ssd_border_destroy(struct ssd *ssd)
{
	assert(ssd);
	assert(ssd->border.tree);

	struct ssd_sub_tree *subtree;
	FOR_EACH_STATE(ssd, subtree) {
		ssd_destroy_parts(&subtree->parts);
		wlr_scene_node_destroy(&subtree->tree->node);
		subtree->tree = NULL;
	} FOR_EACH_END

	wlr_scene_node_destroy(&ssd->border.tree->node);
	ssd->border.tree = NULL;
}

#undef FOR_EACH_STATE
