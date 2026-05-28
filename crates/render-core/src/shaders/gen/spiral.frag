// Logarithmic spiral arms winding into the centre.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float a = atan(p.y, p.x);
    float r = length(p);
    float arms = 2.0 + floor(u_p1 * 8.0);
    float v = 0.5 + 0.5 * sin(arms * a + log(r + 0.05) * 6.0 * u_scale - t * 3.0);
    v = pow(v, mix(1.0, 4.0, u_p2));
    vec3 col = av_hsv(u_hue + r * 0.3, 0.8, v);
    FRAG_COLOR = vec4(col, 1.0);
}
