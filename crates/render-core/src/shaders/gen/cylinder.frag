// A textured cylinder tunnel: angle maps around, 1/r maps into depth.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float a = atan(p.y, p.x) / PI;
    float r = length(p);
    float depth = 1.0 / (r + 0.1) + t;
    float c = mod(floor(a * u_scale * 3.0) + floor(depth * u_scale * 2.0), 2.0);
    float fog = clamp(r * 1.5, 0.0, 1.0);
    vec3 col = mix(av_hsv(u_hue, 0.7, 0.95), av_hsv(u_hue + 0.5, 0.7, 0.3), c) * fog;
    FRAG_COLOR = vec4(col, 1.0);
}
