// Animated Voronoi cells with glowing edges.
void main() {
    vec2 p = av_coord() * u_scale * 3.0;
    float t = av_t();
    vec2 g = floor(p);
    vec2 f = fract(p);
    float d = 8.0;
    float id = 0.0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 o = vec2(float(x), float(y));
            float hh = av_hash(g + o);
            vec2 pt = o + 0.5 + 0.45 * sin(t * 0.8 + PI * vec2(hh, fract(hh * 7.3)));
            float dd = length(f - pt);
            if (dd < d) {
                d = dd;
                id = hh;
            }
        }
    }
    float edge = smoothstep(0.0, 0.09, d);
    vec3 col = av_hsv(u_hue + id, 0.7, mix(1.0, 0.25, edge)) * (1.0 - edge * u_p1);
    FRAG_COLOR = vec4(col, 1.0);
}
