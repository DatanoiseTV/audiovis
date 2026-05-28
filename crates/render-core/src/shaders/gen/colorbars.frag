// SMPTE-flavoured colour bars: a nod to test cards and broadcast monitors.
void main() {
    vec2 uv = v_uv;
    float n = 7.0;
    float i = floor((1.0 - uv.y > 0.25 ? uv.x : uv.x * 0.6 + 0.2) * n);
    float hue = i / n + u_hue;
    float val = (1.0 - uv.y) > 0.25 ? 1.0 : 0.35;
    float sat = (1.0 - uv.y) > 0.25 ? 0.9 : 0.5;
    vec3 col = av_hsv(hue, sat, val);
    // Subtle moving scan so it is clearly "live", scaled by warp.
    col *= 1.0 - u_warp * 0.3 * step(0.5, fract(uv.y * 80.0 + av_t()));
    FRAG_COLOR = vec4(col, 1.0);
}
