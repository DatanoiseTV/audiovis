// A folding-space fractal (Sierpinski-flavoured), slowly rotating.
void main() {
    vec2 p = av_coord() * 0.5 / u_scale + 0.5;
    float t = av_t();
    float v = 0.0;
    for (int i = 0; i < 7; i++) {
        p = abs(p * 2.0 - 1.0);     // fold into the unit cell
        p = av_rot(t * 0.05) * p;
        v += 1.0 / (length(p) + 0.001);
    }
    v = fract(v * 0.05);
    vec3 col = av_hsv(u_hue + v, 0.7, 0.4 + 0.6 * v);
    FRAG_COLOR = vec4(col, 1.0);
}
