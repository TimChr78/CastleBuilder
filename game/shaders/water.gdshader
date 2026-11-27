// Keepwright — lightweight procedural water surface.
//
// A single large plane under each site provides the lake, sea and river. The
// shader animates subtle ripple normals, mixes shallow/deep water colour from
// the opaque depth buffer, adds a Fresnel sky reflection and paints shore foam
// where the water is shallow. It samples no colour textures, so it stays cheap
// even at full-screen coverage.

shader_type spatial;
render_mode blend_mix, cull_back, depth_draw_never, diffuse_burley, specular_schlick_ggx;

uniform vec4 shallow_color : source_color = vec4(0.16, 0.42, 0.40, 1.0);
uniform vec4 deep_color : source_color = vec4(0.05, 0.18, 0.23, 1.0);
uniform vec4 sky_color : source_color = vec4(0.62, 0.72, 0.86, 1.0);
uniform vec4 foam_color : source_color = vec4(0.84, 0.88, 0.84, 1.0);

// World-space ripple field: a few crossed sine waves, no texture fetches.
uniform float ripple_scale = 0.16;
uniform float ripple_speed = 0.55;
uniform float ripple_strength = 0.30;

// Depth response. `deep_distance` is the water depth (metres) at which the
// deep colour fully takes over; `foam_distance` is the shallow band that
// carries the shore foam.
uniform float deep_distance = 14.0;
uniform float foam_distance = 1.4;
uniform float fresnel_power = 4.0;
uniform float min_alpha = 0.62;
uniform float max_alpha = 0.94;

uniform sampler2D depth_texture : hint_depth_texture, repeat_disable, filter_nearest;

varying vec3 v_world_pos;
varying vec3 v_world_normal;

// Cheap crossing wave field. Values are in [-1, 1].
float wave_field(vec2 p, float t) {
	float w = 0.0;
	w += sin(dot(p, vec2(0.80, 0.60)) * 1.7 + t * 1.10);
	w += 0.50 * sin(dot(p, vec2(-0.50, 0.86)) * 3.1 - t * 1.60);
	w += 0.25 * sin(dot(p, vec2(0.94, -0.34)) * 5.3 + t * 2.30);
	return w;
}

void vertex() {
	v_world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	v_world_normal = normalize(MODEL_NORMAL_MATRIX * NORMAL);
}

void fragment() {
	vec3 n = normalize(v_world_normal);

	// Animate a small normal perturbation from the same crossing waves.
	vec2 p = v_world_pos.xz;
	float t = TIME * ripple_speed;
	float step_m = 0.35;
	float h0 = wave_field(p * ripple_scale, t);
	float hx = wave_field((p + vec2(step_m, 0.0)) * ripple_scale, t);
	float hz = wave_field((p + vec2(0.0, step_m)) * ripple_scale, t);
	n = normalize(n + vec3(h0 - hx, 0.0, h0 - hz) * ripple_strength);

	// Opaque depth under this water fragment, in metres.
	float raw_depth = texture(depth_texture, SCREEN_UV).x;
	vec3 ndc = vec3(SCREEN_UV, raw_depth) * 2.0 - 1.0;
	vec4 view = INV_PROJECTION_MATRIX * vec4(ndc, 1.0);
	view.xyz /= view.w;
	float bottom_depth = -view.z;
	float surface_depth = -VERTEX.z;
	float water_depth = max(bottom_depth - surface_depth, 0.0);
	float depth_t = clamp(water_depth / deep_distance, 0.0, 1.0);

	// Shallow-to-deep tint, with a darker rim where the water first meets land.
	vec3 water = mix(shallow_color.rgb, deep_color.rgb, smoothstep(0.0, 1.0, depth_t));
	water *= mix(0.72, 1.0, smoothstep(0.0, 5.0, water_depth));

	// Fresnel sky reflection: strong at grazing angles, faint looking down.
	vec3 view_dir = normalize(CAMERA_POSITION_WORLD - v_world_pos);
	float fresnel = pow(1.0 - clamp(dot(n, view_dir), 0.0, 1.0), fresnel_power);
	fresnel = mix(0.04, 1.0, fresnel);
	vec3 sky = sky_color.rgb;
	vec3 col = mix(water, sky, fresnel * 0.55);

	// Shore foam where the bottom rises into the shallow band.
	float foam = 1.0 - smoothstep(0.0, foam_distance, water_depth);
	float foam_noise = wave_field(p * 1.9, t * 0.8) * 0.5 + 0.5;
	foam *= 0.60 + 0.40 * foam_noise;
	col = mix(col, foam_color.rgb, clamp(foam, 0.0, 1.0) * 0.55);

	ALBEDO = col;
	ROUGHNESS = mix(0.08, 0.30, depth_t);
	SPECULAR = 0.65;
	METALLIC = 0.0;
	ALPHA = clamp(mix(min_alpha, max_alpha, max(depth_t, fresnel)) + foam * 0.18, 0.0, 0.96);

	NORMAL = normalize((VIEW_MATRIX * vec4(n, 0.0)).xyz);
}
