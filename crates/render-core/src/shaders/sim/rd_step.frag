// Gray-Scott reaction-diffusion step. State holds two chemicals: R = A, G = B.
// Feed/kill come from the layer's Param 1/2 knobs (so the whole regime - dots,
// stripes, mazes, mitosis - is modulatable), and the bass keeps injecting B.
uniform sampler2D u_state;
uniform vec2 u_texel;

vec2 lap_sample(vec2 uv) {
    return TEX2D(u_state, uv).rg;
}

void main() {
    vec2 uv = v_uv;
    vec2 s = lap_sample(uv);

    // 9-tap Laplacian.
    vec2 lap = vec2(0.0);
    lap += lap_sample(uv + vec2(-u_texel.x, 0.0)) * 0.2;
    lap += lap_sample(uv + vec2(u_texel.x, 0.0)) * 0.2;
    lap += lap_sample(uv + vec2(0.0, -u_texel.y)) * 0.2;
    lap += lap_sample(uv + vec2(0.0, u_texel.y)) * 0.2;
    lap += lap_sample(uv + vec2(-u_texel.x, -u_texel.y)) * 0.05;
    lap += lap_sample(uv + vec2(u_texel.x, -u_texel.y)) * 0.05;
    lap += lap_sample(uv + vec2(-u_texel.x, u_texel.y)) * 0.05;
    lap += lap_sample(uv + vec2(u_texel.x, u_texel.y)) * 0.05;
    lap -= s;

    float a = s.x;
    float b = s.y;
    float feed = mix(0.030, 0.058, u_p1);
    float kill = mix(0.056, 0.067, u_p2);
    float reaction = a * b * b;
    float na = a + (1.0 * lap.x - reaction + feed * (1.0 - a));
    float nb = b + (0.5 * lap.y + reaction - (kill + feed) * b);

    // Bass sprinkles fresh B so the pattern keeps regenerating live.
    nb += u_audio.x * 0.5 * step(0.997, av_hash(uv * u_res + floor(u_time * 20.0)));

    FRAG_COLOR = vec4(clamp(na, 0.0, 1.0), clamp(nb, 0.0, 1.0), 0.0, 1.0);
}
