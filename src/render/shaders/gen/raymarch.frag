// A small raymarched SDF scene (a rotating torus) - the 3D "wow". Step count is
// kept modest so it still survives on a weak GPU; lower render-scale on a Pi.
float sd_torus(vec3 p, vec2 tt) {
    vec2 q = vec2(length(p.xz) - tt.x, p.y);
    return length(q) - tt.y;
}

void main() {
    vec2 uv = av_coord();
    float t = av_t();
    vec3 ro = vec3(0.0, 0.0, -3.2);
    vec3 rd = normalize(vec3(uv, 1.6));
    mat2 ry = av_rot(t * 0.3);
    mat2 rx = av_rot(t * 0.2 + u_warp * 3.0);

    float d = 0.0;
    float hit = 0.0;
    float steps = 0.0;
    for (int i = 0; i < 48; i++) {
        vec3 p = ro + rd * d;
        p.xz = ry * p.xz;
        p.xy = rx * p.xy;
        float s = sd_torus(p, vec2(1.0 + u_scale * 0.3, 0.35 * (1.0 + u_p1)));
        if (s < 0.002) { hit = 1.0; break; }
        d += s;
        steps += 1.0;
        if (d > 12.0) break;
    }

    vec3 col = vec3(0.0);
    if (hit > 0.5) {
        float glow = 1.0 - d / 12.0;
        float ao = 1.0 - steps / 48.0;
        col = av_hsv(u_hue + d * 0.08, 0.7, glow * (0.4 + 0.6 * ao));
    }
    FRAG_COLOR = vec4(col, 1.0);
}
