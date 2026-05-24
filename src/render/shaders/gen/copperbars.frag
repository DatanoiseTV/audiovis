// Copper bars: glowing horizontal bands sweeping vertically, Amiga demo style.
void main() {
    vec2 uv = v_uv;
    float t = av_t();
    vec3 col = vec3(0.0);
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        float y = 0.5 + 0.42 * sin(t * 0.7 + fi * 1.1 + u_p1 * 6.2831);
        float d = abs(uv.y - y);
        float bar = smoothstep(0.1, 0.0, d);
        float shade = clamp(1.0 - d * 7.0, 0.0, 1.0); // metallic falloff
        col += av_hsv(u_hue + fi * 0.13, 0.85, 1.0) * bar * shade;
    }
    FRAG_COLOR = vec4(col, 1.0);
}
