// Mirror-folded kaleidoscope over a moving plasma/noise base.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float seg = 3.0 + floor(u_p1 * 9.0);
    float a = atan(p.y, p.x);
    float r = length(p);
    a = mod(a, PI / seg);
    a = abs(a - PI / (2.0 * seg));
    p = vec2(cos(a), sin(a)) * r * u_scale * 3.0;

    float v = sin(p.x * 3.0 + t) + sin(p.y * 3.0 - t) + u_warp * 3.0 * av_fbm(p + t * 0.2);
    vec3 col = av_hsv(u_hue + v * 0.1, 0.8, 0.5 + 0.5 * sin(v + r * 4.0));
    FRAG_COLOR = vec4(col, 1.0);
}
