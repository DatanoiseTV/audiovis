// Curl-noise smoke: advect the dye field along a divergence-free flow, dissipate
// slightly, and inject coloured dye (more on the bass). State RGB = dye colour.
uniform sampler2D u_state;
uniform vec2 u_texel;

// Curl of an fBm scalar field -> swirly, mass-conserving velocity.
vec2 curl(vec2 p) {
    float e = 0.01;
    float n1 = av_fbm(p + vec2(0.0, e));
    float n2 = av_fbm(p - vec2(0.0, e));
    float n3 = av_fbm(p + vec2(e, 0.0));
    float n4 = av_fbm(p - vec2(e, 0.0));
    return vec2(n1 - n2, -(n3 - n4)) / (2.0 * e);
}

void main() {
    vec2 uv = v_uv;
    vec2 vel = curl(uv * (2.0 + u_scale * 2.0) + u_time * 0.08) * (0.6 + u_warp * 1.2);
    vec2 prev = uv - vel * u_texel * 6.0;
    vec3 dye = TEX2D(u_state, prev).rgb * 0.99; // slow dissipation

    // Inject sparkles of dye; bass opens the tap.
    float inj = step(0.985, av_hash(uv * u_res + floor(u_time * 24.0))) * (0.25 + u_audio.x * 2.5);
    dye += av_hsv(u_hue + uv.x * 0.5 + u_time * 0.05, 0.8, 1.0) * inj;

    FRAG_COLOR = vec4(min(dye, vec3(1.5)), 1.0);
}
