// A rotating sphere of "bobs" (sprite dots) - the classic demo object.
void main() {
    vec2 uv = av_coord();
    float t = av_t();
    vec3 col = vec3(0.0);
    const float n = 24.0;
    for (int i = 0; i < 24; i++) {
        float fi = float(i);
        float ph = fi * 2.39996323;        // even spread on the sphere
        float y = 1.0 - (fi / n) * 2.0;
        float rr = sqrt(max(0.0, 1.0 - y * y));
        vec3 q = vec3(cos(ph) * rr, y, sin(ph) * rr);
        q.xz = av_rot(t) * q.xz;
        q.yz = av_rot(t * 0.6) * q.yz;
        vec2 sp = q.xy * (0.6 + u_scale * 0.2);
        float depth = 0.5 + 0.5 * q.z;     // front bobs brighter
        col += av_hsv(u_hue + fi * 0.04, 0.7, 1.0) * smoothstep(0.06, 0.0, length(uv - sp)) * depth;
    }
    FRAG_COLOR = vec4(col, 1.0);
}
