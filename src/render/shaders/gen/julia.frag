// Julia set with a constant `c` orbiting, so the fractal morphs continuously.
void main() {
    vec2 z = av_coord() * 1.5 / u_scale;
    float t = av_t() * 0.3;
    vec2 c = 0.7885 * vec2(cos(t), sin(t));
    float it = 0.0;
    for (int i = 0; i < 64; i++) {
        z = vec2(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
        if (dot(z, z) > 4.0) break;
        it += 1.0;
    }
    float m = it / 64.0;
    vec3 col = av_hsv(u_hue + m + u_p1, 0.85, m >= 1.0 ? 0.05 : 1.0);
    FRAG_COLOR = vec4(col, 1.0);
}
