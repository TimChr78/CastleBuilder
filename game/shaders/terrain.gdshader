// Keepwright — terrain splat for M0a.
//
// Blends lush hillside grass, rocky scree and exposed pale limestone bedrock
// using keep-core's baked height/bedrock/slope vertex fields plus world-space
// slope, with procedural macro variation and a cheap normal perturbation so the
// heightfield reads as ground rather than a low-poly sheet. Fed directly from
// the keep-core heightfield mesh.

shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx;

uniform vec4 grass_lush : source_color = vec4(0.22, 0.34, 0.12, 1.0);
uniform vec4 grass_dry : source_color = vec4(0.45, 0.43, 0.20, 1.0);
uniform vec4 scree_color : source_color = vec4(0.46, 0.43, 0.38, 1.0);
uniform vec4 bedrock_color : source_color = vec4(0.75, 0.71, 0.61, 1.0);
uniform vec4 lichen_color : source_color = vec4(0.55, 0.58, 0.38, 1.0);

uniform float grass_slope_limit : hint_range(0.0, 1.0) = 0.30;
uniform float scree_slope : hint_range(0.0, 1.0) = 0.45;
uniform float bedrock_slope : hint_range(0.0, 1.0) = 0.68;
uniform float bedrock_elevation_gain : hint_range(0.0, 1.0) = 0.35;
uniform float macro_scale = 0.06;
uniform float detail_scale = 1.0;
uniform float bump_strength : hint_range(0.0, 2.0) = 0.45;

// keep-core bakes the authoritative terrain fields into vertex colour:
//   r = normalised height, g = bedrock mask, b = slope.
varying vec3 v_world_pos;
varying vec3 v_world_normal;
varying vec4 v_color;

float hash12(vec2 p) {
	vec3 p3 = fract(vec3(p.xyx) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}

float value_noise2(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	float a = hash12(i);
	float b = hash12(i + vec2(1.0, 0.0));
	float c = hash12(i + vec2(0.0, 1.0));
	float d = hash12(i + vec2(1.0, 1.0));
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm2(vec2 p) {
	float sum = 0.0;
	float amp = 0.5;
	for (int i = 0; i < 4; i++) {
		sum += amp * value_noise2(p);
		p *= 2.07;
		amp *= 0.5;
	}
	return sum;
}

void vertex() {
	v_world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	v_world_normal = normalize(MODEL_NORMAL_MATRIX * NORMAL);
	v_color = COLOR;
}

void fragment() {
	vec3 n = normalize(v_world_normal);

	// Authoritative fields straight from keep-core's heightfield mesh.
	float baked_height = clamp(v_color.r, 0.0, 1.0);
	float baked_bedrock = clamp(v_color.g, 0.0, 1.0);
	float baked_slope = clamp(v_color.b, 0.0, 1.0);

	float geo_slope = 1.0 - clamp(n.y, 0.0, 1.0);
	float slope = max(geo_slope, baked_slope);
	float elevation = baked_height;

	float macro = fbm2(v_world_pos.xz * macro_scale + vec2(13.0, 7.0));
	float detail = fbm2(v_world_pos.xz * (0.90 * detail_scale) + v_world_pos.y * 0.15);
	float rock_noise = fbm2(v_world_pos.xz * 2.30 + vec2(3.1, 9.4));

	// Slope bands. Grass yields to scree, scree yields to bedrock on the steeps.
	float grass_mask = 1.0 - smoothstep(grass_slope_limit, grass_slope_limit + 0.16, slope);
	grass_mask *= 1.0 - smoothstep(0.72, 1.0, elevation) * 0.60;
	grass_mask = clamp(grass_mask + (macro - 0.5) * 0.28, 0.0, 1.0);

	float scree_mask = smoothstep(grass_slope_limit * 0.80, scree_slope, slope);
	scree_mask *= 1.0 - smoothstep(scree_slope + 0.12, 0.95, slope);

	float bedrock_mask = smoothstep(bedrock_slope, min(bedrock_slope + 0.22, 0.98), slope);
	bedrock_mask = max(bedrock_mask, smoothstep(0.55, 1.0, elevation) * bedrock_elevation_gain * smoothstep(0.20, 0.60, slope));
	// keep-core's bedrock mask is authoritative: never let the procedural guess
	// hide rock the heightfield has already exposed.
	bedrock_mask = max(bedrock_mask, baked_bedrock);
	bedrock_mask = clamp(bedrock_mask, 0.0, 1.0);

	vec3 grass = mix(grass_lush.rgb, grass_dry.rgb, clamp(macro * 0.80 + elevation * 0.35, 0.0, 1.0));
	grass *= 0.82 + 0.38 * detail;

	vec3 scree = scree_color.rgb * (0.80 + 0.40 * rock_noise) * (0.85 + 0.25 * detail);

	vec3 bedrock = bedrock_color.rgb * (0.82 + 0.30 * rock_noise);
	bedrock = mix(bedrock, lichen_color.rgb, smoothstep(0.55, 0.85, macro) * 0.30);

	vec3 albedo = grass;
	albedo = mix(albedo, scree, clamp(scree_mask, 0.0, 1.0));
	albedo = mix(albedo, bedrock, bedrock_mask);

	ALBEDO = albedo;
	ROUGHNESS = mix(mix(0.95, 0.82, scree_mask), 0.72, bedrock_mask);
	SPECULAR = 0.25;
	METALLIC = 0.0;

	// Subtle procedural bump from the same noise field, strongest on rock.
	float h0 = fbm2(v_world_pos.xz * 3.0);
	float hx = fbm2((v_world_pos.xz + vec2(0.05, 0.0)) * 3.0);
	float hz = fbm2((v_world_pos.xz + vec2(0.0, 0.05)) * 3.0);
	vec3 up_ref = abs(n.y) > 0.95 ? vec3(0.0, 0.0, 1.0) : vec3(0.0, 1.0, 0.0);
	vec3 tangent = normalize(cross(n, up_ref));
	vec3 bitangent = cross(tangent, n);
	float rock_weight = max(scree_mask, bedrock_mask);
	float bump = bump_strength * (0.25 + 0.75 * rock_weight) * (1.0 - grass_mask * 0.65);
	vec3 bumped = normalize(n + (tangent * (h0 - hx) + bitangent * (h0 - hz)) * bump * 6.0);

	NORMAL = normalize((VIEW_MATRIX * vec4(bumped, 0.0)).xyz);
}
