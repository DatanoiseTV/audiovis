// Mandelbrot set, slowly cycling its zoom into a seahorse-valley point.
void main() {
    vec2 p = av_coord();
    float zoom = exp(-mod(av_t() * 0.1, 4.0));
    vec2 c = vec2(-0.745, 0.113) + p * zoom * 1.5 / u_scale;
    vec2 z = vec2(0.0);
    float it = 0.0;
    for (int i = 0; i < 64; i++) {
        z = vec2(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
        if (dot(z, z) > 4.0) break;
        it += 1.0;
    }
    float m = it / 64.0;
    vec3 col = m >= 1.0 ? vec3(0.0) : av_hsv(u_hue + m * 2.0 + u_p1, 0.85, 1.0);
    FRAG_COLOR = vec4(col, 1.0);
}
