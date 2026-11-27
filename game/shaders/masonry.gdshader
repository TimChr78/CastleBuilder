// Keepwright — limestone ashlar masonry.
//
// Grounded painterly realism at golden hour. Rendering contract with keep-gd:
//   UV      : local block-face UV in [0, 1]
//   COLOR   : r = baked sky/hemisphere AO, g = warm/cool tint, b = age, a = jitter seed
//   CUSTOM0 : r = build_index_norm [0,1], g = course_index, b = block width (m),
//             a = block height (m), negative when the block is a roof tile
//
// Bevelled edges and mortar joints are produced procedurally from UV, so ordinary
// 6-10 triangle blocks read as crisp dressed stone without extra geometry.
// The construction front is a single uniform; no per-frame vertex uploads.

shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

// --- Limestone palette -------------------------------------------------
uniform vec4 stone_light : source_color = vec4(0.80, 0.75, 0.63, 1.0);
uniform vec4 stone_warm : source_color = vec4(0.69, 0.59, 0.44, 1.0);
uniform vec4 stone_cool : source_color = vec4(0.55, 0.58, 0.57, 1.0);
uniform vec4 mortar_color : source_color = vec4(0.47, 0.44, 0.38, 1.0);
// 0 = limestone ashlar, 1 = Baltic weathered red brick.
uniform int material_palette = 0;
// --- Baltic red brick palette ------------------------------------------
uniform vec4 brick_light : source_color = vec4(0.56, 0.24, 0.17, 1.0);
uniform vec4 brick_warm : source_color = vec4(0.48, 0.20, 0.14, 1.0);
uniform vec4 brick_cool : source_color = vec4(0.42, 0.17, 0.13, 1.0);
uniform vec4 brick_mortar : source_color = vec4(0.76, 0.72, 0.65, 1.0);
// --- Medieval roof slate (shared by both palettes) ---------------------
uniform vec4 slate_light : source_color = vec4(0.18, 0.21, 0.25, 1.0);
uniform vec4 slate_dark : source_color = vec4(0.11, 0.13, 0.16, 1.0);
uniform vec4 sky_fill_color : source_color = vec4(0.33, 0.45, 0.63, 1.0);
uniform vec4 bounce_color : source_color = vec4(0.96, 0.66, 0.36, 1.0);

// Warm key direction used only for the baked-looking bounce term. This is a
// stylised fill, not a real second light; it keeps the wall readable without GI.
uniform vec3 sun_bounce_dir = vec3(-0.45, 0.35, 0.82);

// --- Construction animation -------------------------------------------
// Blocks whose build_index_norm is at or ahead of build_front are collapsed
// below the ground; those just behind the front drop and settle into place.
uniform float build_front : hint_range(0.0, 1.0) = 1.0;
uniform float build_feather : hint_range(0.001, 0.30) = 0.05;
uniform float build_drop_height : hint_range(0.0, 4.0) = 1.2;
// Blocks well ahead of the front are sunk far below the site so they read as
// hidden without alpha tests, keeping the shadow pass cheap and correct.
uniform float build_hide_depth : hint_range(0.0, 1000.0) = 400.0;

// --- Surface detail (metres) ------------------------------------------
uniform float bevel_width : hint_range(0.0, 0.20) = 0.045;
uniform float bevel_strength : hint_range(0.0, 2.0) = 0.90;
uniform float mortar_width : hint_range(0.0, 0.05) = 0.012;
uniform float grain_scale : hint_range(0.1, 64.0) = 11.0;
uniform float grain_strength : hint_range(0.0, 1.0) = 0.28;
uniform float weathering_strength : hint_range(0.0, 2.0) = 0.80;
uniform float sky_fill_strength : hint_range(0.0, 2.0) = 0.60;
uniform float bounce_strength : hint_range(0.0, 2.0) = 0.45;
// Ground level for the wet/damp course at the foot of the wall (metres).
uniform float wet_base_y : hint_range(-50.0, 100.0) = 0.0;
// Height over which dampness and moss fade out above the base (metres).
uniform float damp_height : hint_range(0.2, 8.0) = 2.2;

// --- LOD shells --------------------------------------------------------
// LOD1/LOD2 are single quads per element face whose UV spans the whole face,
// so the per-block UV pipeline above would paint one giant slab with a dark
// border at every chunk seam. Those LODs instead recompose coursed ashlar in
// world space, which also keeps courses continuous across chunk boundaries.
// 0 = LOD0 full blocks (per-face UV), 1 = LOD1 shell, 2 = LOD2 massing proxy.
instance uniform int lod_level = 0;
// Defaults match keep-core's document course height (450 mm) and nominal
// block width (950 mm), so the virtual courses line up with LOD0's real ones.
uniform float lod_course_height : hint_range(0.10, 1.00) = 0.45;
uniform float lod_block_width : hint_range(0.20, 2.00) = 0.95;

varying vec3 v_world_pos;
varying vec3 v_world_normal;
varying vec4 v_color;
varying vec4 v_custom;

// --- Cheap procedural noise -------------------------------------------
float hash13(vec3 p3) {
	p3 = fract(p3 * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

// Deterministic per-cell hash used by the LOD shell's virtual stonework.
float hash21(vec2 p) {
	p = fract(p * vec2(123.34, 456.21));
	p += dot(p, p + 45.32);
	return fract(p.x * p.y);
}

float value_noise3(vec3 p) {
	vec3 i = floor(p);
	vec3 f = fract(p);
	vec3 u = f * f * (3.0 - 2.0 * f);

	float n000 = hash13(i + vec3(0.0, 0.0, 0.0));
	float n100 = hash13(i + vec3(1.0, 0.0, 0.0));
	float n010 = hash13(i + vec3(0.0, 1.0, 0.0));
	float n110 = hash13(i + vec3(1.0, 1.0, 0.0));
	float n001 = hash13(i + vec3(0.0, 0.0, 1.0));
	float n101 = hash13(i + vec3(1.0, 0.0, 1.0));
	float n011 = hash13(i + vec3(0.0, 1.0, 1.0));
	float n111 = hash13(i + vec3(1.0, 1.0, 1.0));

	float nx00 = mix(n000, n100, u.x);
	float nx10 = mix(n010, n110, u.x);
	float nx01 = mix(n001, n101, u.x);
	float nx11 = mix(n011, n111, u.x);
	float nxy0 = mix(nx00, nx10, u.y);
	float nxy1 = mix(nx01, nx11, u.y);
	return mix(nxy0, nxy1, u.z);
}

float fbm3(vec3 p) {
	float sum = 0.0;
	float amp = 0.5;
	for (int i = 0; i < 4; i++) {
		sum += amp * value_noise3(p);
		p *= 2.03;
		amp *= 0.5;
	}
	return sum;
}

// Gentle weathering shared by both LOD pipelines:
//   * damp darkening in the lowest ~2 m, strongest where the wall meets soil;
//   * a cool, faintly mossy cast on north-facing masonry;
//   * a warm fresh-mortar blush just behind the active construction lift.
vec3 weathered_stone(vec3 albedo, vec3 wp, vec3 wn, float fresh_lift) {
	float above_base = max(wp.y - wet_base_y, 0.0);
	float damp = 1.0 - smoothstep(0.0, damp_height, above_base);
	float north = smoothstep(0.0, 0.75, -wn.z);
	float moss = damp * north;

	albedo *= 1.0 - damp * 0.15 * weathering_strength;
	albedo = mix(
		albedo,
		albedo * vec3(0.86, 0.97, 0.90),
		north * 0.30 * weathering_strength
	);
	albedo *= 1.0 - moss * 0.10 * weathering_strength;
	albedo = mix(
		albedo,
		albedo * vec3(1.05, 1.01, 0.95) + vec3(0.045, 0.040, 0.030),
		fresh_lift * 0.60
	);
	return albedo;
}

void vertex() {
	// Chunk meshes arrive in world space and instances stay at the identity
	// transform, so VERTEX is already in world coordinates here.

	// GPU construction front. block_index is normalised across the whole build.
	float block_index = CUSTOM0.r;
	float reveal = smoothstep(block_index - build_feather, block_index, build_front);
	float drop = 1.0 - reveal;

	// Collapse straight down, with a tiny settle sway as the block lands.
	// The sharp power term sinks not-yet-built blocks far below the terrain.
	float sink = pow(drop, 16.0) * build_hide_depth;
	VERTEX.y -= drop * drop * build_drop_height + sink;
	VERTEX.x += sin(reveal * 3.14159265) * 0.02 * drop;

	v_world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	v_world_normal = normalize((MODEL_NORMAL_MATRIX * NORMAL));
	v_color = COLOR;
	v_custom = CUSTOM0;
}

void fragment() {
	// Narrow warm band of freshly laid mortar just behind the active lift.
	// Disabled as soon as the build finishes so the completed wall reads calm.
	float block_index = v_custom.r;
	float fresh_lift = 0.0;
	if (build_front < 0.995) {
		float behind = build_front - block_index;
		fresh_lift = (1.0 - smoothstep(0.0, 0.08, behind)) * step(0.0, behind);
	}

	if (lod_level >= 1) {
		// --- World-space coursed ashlar for simplified LOD shells ---------
		vec3 wp = v_world_pos;
		vec3 wn = normalize(v_world_normal);

		float grain = fbm3(wp * grain_scale);
		float patches = fbm3(wp * (grain_scale * 0.17) + vec3(7.1, 3.7, 11.3));
		float streak_field = fbm3(vec3(wp.x * 1.8, wp.y * 0.22, wp.z * 1.8) + 5.0);
		float age = clamp(v_color.b, 0.0, 1.0);
		float streaks = smoothstep(0.48, 0.86, streak_field) * (0.35 + age * 0.65);

		// Pick the horizontal running axis per pixel. Both vertical projections
		// share the same world-Y row phase, so courses stay level across the
		// blend on oblique tower facets; near-horizontal caps switch to a plan
		// projection so they read as flagstones rather than stripes.
		float along_x = step(abs(wn.z), abs(wn.x));
		float horiz = smoothstep(0.55, 0.95, abs(wn.y));
		float u_m = mix(mix(wp.x, wp.z, along_x), wp.x, horiz);
		float v_m = mix(wp.y, wp.z, horiz);

		// Running bond: alternating courses shift by half a block.
		float row_f = v_m / lod_course_height;
		float row = floor(row_f);
		float col_f = u_m / lod_block_width + mod(row, 2.0) * 0.5;
		float col = floor(col_f);
		vec2 cfrac = vec2(fract(col_f), fract(row_f));
		vec2 cell = vec2(col, row);

		float edge_m = min(
			min(cfrac.x, 1.0 - cfrac.x) * lod_block_width,
			min(cfrac.y, 1.0 - cfrac.y) * lod_course_height
		);

		// Metres covered by one pixel along the coursing axes. Everything below
		// fades detail once joints/bevels fall below a pixel, so far LOD2 shells
		// soften into warm limestone instead of moiré-shimmering.
		float aa_m = max(
			fwidth(col_f) * lod_block_width, fwidth(row_f) * lod_course_height
		);
		aa_m = max(aa_m, 1e-4);

		float joint_visible = 1.0 - smoothstep(1.0, 3.0, aa_m / max(mortar_width, 1e-5));
		float mortar = 1.0 - smoothstep(
			mortar_width * 0.5, mortar_width * 0.5 + aa_m * 0.75, edge_m
		);
		mortar *= joint_visible;

		float bevel_visible = 1.0 - smoothstep(0.5, 1.5, aa_m / max(bevel_width, 1e-5));
		float bevel = 1.0 - smoothstep(0.0, max(bevel_width, aa_m), edge_m);
		bevel *= bevel_visible;

		// Bevel tangent frame follows the same projection as the coursing.
		vec3 axis_u = mix(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), along_x);
		axis_u = mix(axis_u, vec3(1.0, 0.0, 0.0), horiz);
		vec3 axis_v = mix(vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0), along_x);
		axis_v = mix(axis_v, vec3(0.0, 0.0, 1.0), horiz);
		vec2 bdir = cfrac * 2.0 - 1.0;
		vec3 bevel_world = axis_u * bdir.x + axis_v * bdir.y;
		// NORMAL is view space in the fragment stage; rotate the world-space
		// bevel direction into it. LOD0's derivative tangent frame does the
		// same job implicitly, but shells have no per-block UVs to derive from.
		vec3 bevel_view = (VIEW_MATRIX * vec4(bevel_world, 0.0)).xyz;
		NORMAL = normalize(NORMAL + bevel_view * bevel * bevel_strength);

		// Per-virtual-stone warm/cool tint and brightness, matching LOD0's
		// 0.15..0.90 tint palette and 0.12 seed brightness variation. Tint
		// cells are block-sized, so they stay meaningful much further out than
		// mortar or grain; only soften once a block is a few pixels across.
		float h_tint = hash21(cell);
		float h_bright = hash21(cell + vec2(31.7, 7.3));
		float far_soft = smoothstep(0.12, 0.50, aa_m);
		float cell_tint = mix(mix(0.15, 0.90, h_tint), 0.52, far_soft);
		float cell_bright = mix(1.0 + (h_bright - 0.5) * 0.12, 1.0, far_soft);

		// Sub-pixel grain would alias on distant shells; fade it to its mean.
		float grain_mix = mix(0.5, grain, 1.0 - smoothstep(0.05, 0.18, aa_m));

		vec3 pal_light = material_palette == 1 ? brick_light.rgb : stone_light.rgb;
		vec3 pal_warm = material_palette == 1 ? brick_warm.rgb : stone_warm.rgb;
		vec3 pal_cool = material_palette == 1 ? brick_cool.rgb : stone_cool.rgb;
		vec3 pal_mortar = material_palette == 1 ? brick_mortar.rgb : mortar_color.rgb;

		vec3 stone = mix(pal_cool, pal_warm, smoothstep(0.12, 0.92, cell_tint));
		stone = mix(pal_light, stone, 0.55);
		stone *= cell_bright;
		stone *= mix(1.0, 0.78 + 0.42 * grain_mix, grain_strength);
		stone *= mix(vec3(1.0), vec3(0.86, 0.90, 0.80), age * 0.55);
		stone *= 1.0 - streaks * 0.32 * weathering_strength;

		vec3 joint = pal_mortar * (0.85 + 0.30 * patches);
		vec3 albedo = mix(stone, joint, mortar * 0.92);
		albedo = weathered_stone(albedo, wp, wn, fresh_lift);

		ALBEDO = albedo;
		ROUGHNESS = clamp(
			0.58 + 0.16 * grain_mix + 0.18 * mortar + 0.12 * age
			- streaks * 0.10 * weathering_strength,
			0.0, 1.0
		);
		SPECULAR = 0.32;
		METALLIC = 0.0;

		float baked_ao = clamp(v_color.r, 0.0, 1.0);
		AO = clamp(baked_ao * (1.0 - 0.45 * mortar), 0.0, 1.0);
		AO_LIGHT_AFFECT = 0.35;

		float up = clamp(wn.y * 0.5 + 0.5, 0.0, 1.0);
		float sky_vis = mix(0.35, 1.0, up);
		float sun_facing = clamp(dot(wn, normalize(sun_bounce_dir)), 0.0, 1.0);
		float ground_facing = clamp(-wn.y, 0.0, 1.0);

		vec3 fill = sky_fill_color.rgb * (sky_fill_strength * sky_vis);
		vec3 bounce = bounce_color.rgb
			* (bounce_strength * (sun_facing * 0.35 + ground_facing * 0.65));
		EMISSION = albedo * (fill + bounce);
	} else {
		// --- LOD0: per-block UV bevel/mortar pipeline (unchanged) ----------
		vec2 uv = UV;
	vec4 custom = v_custom;

	// Blocks may be different sizes, so convert UV distance to metres using the
	// per-block dimensions. Roof tiles encode their height as a negative value,
	// so read the magnitude and keep the sign as an explicit roof flag.
	float block_height_m = abs(custom.a);
	float dims_valid = step(0.001, custom.b) * step(0.001, block_height_m);
	vec2 block_size_m = mix(vec2(0.40, 0.25), vec2(custom.b, block_height_m), dims_valid);
	vec2 edge_uv = min(uv, vec2(1.0) - uv);
	vec2 edge_m = edge_uv * block_size_m;
	float edge_distance = min(edge_m.x, edge_m.y);

	// Analytical band-limiting. fwidth() gives the UV footprint of one pixel;
	// multiplying by the block size converts it to metres. At grazing angles and
	// 40-90 m the joint/bevel/grain features fall below a pixel, so they fade to
	// their mean instead of shimmering (the same contract LOD1 enforces).
	vec2 uv_footprint = vec2(
		length(vec2(dFdx(uv.x), dFdy(uv.x))),
		length(vec2(dFdx(uv.y), dFdy(uv.y)))
	);
	vec2 aa_m = max(uv_footprint * block_size_m, vec2(1e-5));
	float aa_edge = max(aa_m.x, aa_m.y);

	float mortar_visible = 1.0 - smoothstep(
		1.0, 3.0, aa_edge / max(mortar_width, 1e-5)
	);
	// Widening the transition by the pixel footprint keeps the joint's true
	// width up close while averaging it away once it falls below a pixel.
	float mortar = 1.0 - smoothstep(
		max(mortar_width - aa_edge, 0.0),
		mortar_width + aa_edge,
		edge_distance
	);
	mortar *= mortar_visible;

	float bevel_visible = 1.0 - smoothstep(
		0.35, 1.0, aa_edge / max(bevel_width, 1e-5)
	);
	float bevel = 1.0 - smoothstep(0.0, max(bevel_width, aa_edge), edge_distance);
	bevel *= bevel_visible;

	// Derivative tangent frame (Mikkelsen-style) so bevel normals work even
	// though chunk meshes deliberately omit tangents.
	vec3 dp1 = dFdx(VERTEX);
	vec3 dp2 = dFdy(VERTEX);
	vec2 duv1 = dFdx(uv);
	vec2 duv2 = dFdy(uv);
	vec3 dp2perp = cross(dp2, NORMAL);
	vec3 dp1perp = cross(NORMAL, dp1);
	vec3 tangent = dp2perp * duv1.x + dp1perp * duv2.x;
	vec3 bitangent = dp2perp * duv1.y + dp1perp * duv2.y;
	// Normalise each axis independently so bevel normals stay equally crisp
	// along the short and long edges of a non-square stone.
	tangent *= inversesqrt(max(dot(tangent, tangent), 1e-12));
	bitangent *= inversesqrt(max(dot(bitangent, bitangent), 1e-12));

	// If UV is constant (mesh without usable UVs), fade the procedural joint
	// shading out rather than flooding the whole block with mortar.
	float uv_span = abs(duv1.x) + abs(duv1.y) + abs(duv2.x) + abs(duv2.y);
	float uv_valid = smoothstep(0.0, 0.0005, uv_span);
	mortar *= uv_valid;
	bevel *= uv_valid;

	vec2 bevel_dir = uv * 2.0 - 1.0;
	vec3 bevel_normal = tangent * bevel_dir.x + bitangent * bevel_dir.y;
	NORMAL = normalize(NORMAL + bevel_normal * bevel * bevel_strength);

	// --- Procedural limestone grain, mottling and weathering streaks ---
	vec3 wp = v_world_pos;
	vec3 wn = normalize(v_world_normal);
	float grain = fbm3(wp * grain_scale);
	float patches = fbm3(wp * (grain_scale * 0.17) + vec3(7.1, 3.7, 11.3));
	float streak_field = fbm3(vec3(wp.x * 1.8, wp.y * 0.22, wp.z * 1.8) + 5.0);
	float age = clamp(v_color.b, 0.0, 1.0);
	float streaks = smoothstep(0.48, 0.86, streak_field) * (0.35 + age * 0.65);

	// Band-limit the procedural fields: once one grain cell (or mottling cell)
	// covers less than a few pixels, blend it back to its mean value. This is
	// what kills the moiré shimmer on distant/grazing LOD0 faces.
	float grain_aa = aa_edge * grain_scale;
	float grain_visible = 1.0 - smoothstep(0.20, 0.60, grain_aa);
	float grain_mix = mix(0.5, grain, grain_visible);
	float patch_aa = aa_edge * grain_scale * 0.17;
	float patches_mix = mix(0.5, patches, 1.0 - smoothstep(0.50, 1.50, patch_aa));

	// --- Per-vertex data ---
	float baked_ao = clamp(v_color.r, 0.0, 1.0);
	float tint = clamp(v_color.g, 0.0, 1.0);
	float seed = clamp(v_color.a, 0.0, 1.0);

	vec3 pal_light = material_palette == 1 ? brick_light.rgb : stone_light.rgb;
	vec3 pal_warm = material_palette == 1 ? brick_warm.rgb : stone_warm.rgb;
	vec3 pal_cool = material_palette == 1 ? brick_cool.rgb : stone_cool.rgb;
	vec3 pal_mortar = material_palette == 1 ? brick_mortar.rgb : mortar_color.rgb;

	vec3 stone = mix(pal_cool, pal_warm, smoothstep(0.12, 0.92, tint));
	stone = mix(pal_light, stone, 0.55);
	stone *= 1.0 + (seed - 0.5) * 0.12;
	stone *= mix(1.0, 0.78 + 0.42 * grain_mix, grain_strength);
	stone *= mix(vec3(1.0), vec3(0.86, 0.90, 0.80), age * 0.55);
	stone *= 1.0 - streaks * 0.32 * weathering_strength;

	vec3 joint = pal_mortar * (0.85 + 0.30 * patches_mix);
	vec3 albedo = mix(stone, joint, mortar * 0.92);

	// Explicit roof flag: only blocks the generator tagged as slate carry the
	// negative encoded height. Short bricks and gatehouse courses stay stone.
	float slate = (custom.a < 0.0) ? 1.0 : 0.0;
	vec3 slate_albedo = mix(slate_dark.rgb, slate_light.rgb, smoothstep(0.12, 0.92, tint));
	slate_albedo *= 0.88 + (seed - 0.5) * 0.24;
	slate_albedo *= mix(1.0, 0.82 + 0.36 * grain_mix, grain_strength * 0.7);
	albedo = mix(albedo, slate_albedo, slate);
	albedo = weathered_stone(albedo, wp, wn, fresh_lift);

	ALBEDO = albedo;
	ROUGHNESS = clamp(0.58 + 0.16 * grain_mix + 0.18 * mortar + 0.12 * age - streaks * 0.10 * weathering_strength, 0.0, 1.0);
	SPECULAR = 0.32;
	METALLIC = 0.0;

	// Baked AO is applied exactly once, through Godot's AO output. Keeping it out
	// of ALBEDO and EMISSION avoids the triple-darkening that crushed shadowed
	// vertical faces to black; AO_LIGHT_AFFECT keeps some sky-fill on them.
	AO = clamp(baked_ao * (1.0 - 0.45 * mortar), 0.0, 1.0);
	AO_LIGHT_AFFECT = 0.35;

	// --- Sky fill + warm directional bounce (no dynamic GI needed) ---
	float up = clamp(wn.y * 0.5 + 0.5, 0.0, 1.0);
	float sky_vis = mix(0.35, 1.0, up);
	float sun_facing = clamp(dot(wn, normalize(sun_bounce_dir)), 0.0, 1.0);
	float ground_facing = clamp(-wn.y, 0.0, 1.0);

	vec3 fill = sky_fill_color.rgb * (sky_fill_strength * sky_vis);
	vec3 bounce = bounce_color.rgb * (bounce_strength * (sun_facing * 0.35 + ground_facing * 0.65));
	EMISSION = albedo * (fill + bounce);
	}
}
