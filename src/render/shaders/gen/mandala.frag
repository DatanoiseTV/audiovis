// Radial mandala: angular symmetry times concentric ripples.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float a = atan(p.y, p.x);
    float r = length(p);
    float sym = floor(6.0 + u_p1 * 12.0);
    float fold = cos(a * sym);
    float v = sin(r * 10.0 * u_scale - t * 2.0) * fold + 0.5 * sin(r * 20.0 + t);
    vec3 col = av_hsv(u_hue + r + v * 0.2, 0.8, 0.5 + 0.5 * sin(v * 3.0 + r * 8.0));
    FRAG_COLOR = vec4(col, 1.0);
}
