// Colourise the reaction-diffusion state: B concentration drives a smooth,
// glowing gradient with a soft edge so it reads as living tissue, not noise.
uniform sampler2D u_state;
uniform vec2 u_texel;

void main() {
    float b = TEX2D(u_state, v_uv).g;
    // Cheap shading from the B gradient gives the pattern relief.
    float bx = TEX2D(u_state, v_uv + vec2(u_texel.x, 0.0)).g - b;
    float by = TEX2D(u_state, v_uv + vec2(0.0, u_texel.y)).g - b;
    float shade = clamp(1.0 - (bx + by) * 12.0, 0.4, 1.4);

    float v = smoothstep(0.05, 0.32, b);
    vec3 col = av_hsv(u_hue + b * 1.6 + 0.55, 0.7, v * shade);
    FRAG_COLOR = vec4(col, 1.0);
}
