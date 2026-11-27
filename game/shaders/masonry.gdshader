// Keepwright — limestone ashlar masonry.
//
// Grounded painterly realism at golden hour. Rendering contract with keep-gd:
//   UV      : local block-face UV in [0, 1]
//   COLOR   : r = baked sky/hemisphere AO, g = warm/cool tint, b = age, a = jitter seed
//   CUSTOM0 : r = build_index_norm [0,1], g = course_index, b = block width (m),
//             a = block height (m)
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
	vec2 uv = UV;
	vec4 custom = v_custom;

	// Blocks may be different sizes, so convert UV distance to metres using the
	// per-block dimensions. Fall back to a sensible size if the attribute is
	// missing (e.g. a future simplified LOD mesh).
	float dims_valid = step(0.001, custom.b) * step(0.001, custom.a);
	vec2 block_size_m = mix(vec2(0.40, 0.25), vec2(custom.b, custom.a), dims_valid);
	vec2 edge_uv = min(uv, vec2(1.0) - uv);
	vec2 edge_m = edge_uv * block_size_m;
	float edge_distance = min(edge_m.x, edge_m.y);

	float mortar = 1.0 - smoothstep(0.0, mortar_width, edge_distance);
	float bevel = 1.0 - smoothstep(0.0, bevel_width, edge_distance);

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
	float grain = fbm3(wp * grain_scale);
	float patches = fbm3(wp * (grain_scale * 0.17) + vec3(7.1, 3.7, 11.3));
	float streak_field = fbm3(vec3(wp.x * 1.8, wp.y * 0.22, wp.z * 1.8) + 5.0);
	float age = clamp(v_color.b, 0.0, 1.0);
	float streaks = smoothstep(0.48, 0.86, streak_field) * (0.35 + age * 0.65);

	// --- Per-vertex data ---
	float baked_ao = clamp(v_color.r, 0.0, 1.0);
	float tint = clamp(v_color.g, 0.0, 1.0);
	float seed = clamp(v_color.a, 0.0, 1.0);

	vec3 stone = mix(stone_cool.rgb, stone_warm.rgb, smoothstep(0.12, 0.92, tint));
	stone = mix(stone_light.rgb, stone, 0.55);
	stone *= 1.0 + (seed - 0.5) * 0.12;
	stone *= mix(1.0, 0.78 + 0.42 * grain, grain_strength);
	stone *= mix(vec3(1.0), vec3(0.86, 0.90, 0.80), age * 0.55);
	stone *= 1.0 - streaks * 0.32 * weathering_strength;

	vec3 joint = mortar_color.rgb * (0.85 + 0.30 * patches);
	vec3 albedo = mix(stone, joint, mortar * 0.92);

	ALBEDO = albedo;
	ROUGHNESS = clamp(0.58 + 0.16 * grain + 0.18 * mortar + 0.12 * age - streaks * 0.10 * weathering_strength, 0.0, 1.0);
	SPECULAR = 0.32;
	METALLIC = 0.0;

	// Baked AO is applied exactly once, through Godot's AO output. Keeping it out
	// of ALBEDO and EMISSION avoids the triple-darkening that crushed shadowed
	// vertical faces to black; AO_LIGHT_AFFECT keeps some sky-fill on them.
	AO = clamp(baked_ao * (1.0 - 0.45 * mortar), 0.0, 1.0);
	AO_LIGHT_AFFECT = 0.35;

	// --- Sky fill + warm directional bounce (no dynamic GI needed) ---
	vec3 wn = normalize(v_world_normal);
	float up = clamp(wn.y * 0.5 + 0.5, 0.0, 1.0);
	float sky_vis = mix(0.35, 1.0, up);
	float sun_facing = clamp(dot(wn, normalize(sun_bounce_dir)), 0.0, 1.0);
	float ground_facing = clamp(-wn.y, 0.0, 1.0);

	vec3 fill = sky_fill_color.rgb * (sky_fill_strength * sky_vis);
	vec3 bounce = bounce_color.rgb * (bounce_strength * (sun_facing * 0.35 + ground_facing * 0.65));
	EMISSION = albedo * (fill + bounce);
}
