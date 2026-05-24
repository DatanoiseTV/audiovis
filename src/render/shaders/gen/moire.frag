// Two rotating high-frequency grids beating against each other.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float s = u_scale * 40.0;
    vec2 q = av_rot(t * 0.1 + u_warp) * p;
    float g1 = sin(p.x * s) * sin(p.y * s);
    float g2 = sin(q.x * s * (1.0 + u_p1)) * sin(q.y * s * (1.0 + u_p1));
    float v = g1 * g2;
    vec3 col = av_hsv(u_hue + 0.5 * v, 0.6, 0.5 + 0.5 * v);
    FRAG_COLOR = vec4(col, 1.0);
}
