// Twister: a column of bobs spiralling up the screen.
void main() {
    vec2 uv = v_uv;
    float t = av_t();
    float twist = t + uv.y * (3.0 + u_p1 * 8.0);
    float v = 0.0;
    for (int i = 0; i < 4; i++) {
        float ang = twist + float(i) * 1.5708;
        float x = 0.5 + 0.32 * cos(ang) * u_scale;
        float depth = 0.4 + 0.6 * (0.5 + 0.5 * sin(ang)); // front faces brighter
        v = max(v, smoothstep(0.045, 0.0, abs(uv.x - x)) * depth);
    }
    vec3 col = av_hsv(u_hue + uv.y * 0.3, 0.75, v);
    FRAG_COLOR = vec4(col, 1.0);
}
