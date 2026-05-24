// Endless radial tunnel: angular spokes times depth rings, audio pushes speed.
void main() {
    vec2 p = av_coord();
    float a = atan(p.y, p.x);
    float r = length(p);
    float t = av_t() + u_audio.x * 2.0;

    float spokes = 2.0 + floor(u_p1 * 16.0);
    float depth = 0.35 / (r + 0.05) + t;
    float bands = 0.5 + 0.5 * sin(a * spokes);
    float rings = 0.5 + 0.5 * sin(depth * PI * u_scale - t * 2.0);
    float v = bands * rings;

    vec3 col = av_hsv(u_hue + depth * 0.04, 0.7, v * clamp(r * 1.4, 0.0, 1.0));
    FRAG_COLOR = vec4(col, 1.0);
}
