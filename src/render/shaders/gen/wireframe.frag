// Raymarched solid morphing sphere -> box -> octahedron -> torus (Param 1),
// shaded as a glowing wireframe lattice (Param 2 = mesh density) on black.
mat3 rot_y(float a) { float c = cos(a), s = sin(a); return mat3(c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c); }
mat3 rot_x(float a) { float c = cos(a), s = sin(a); return mat3(1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c); }

float map(vec3 p) {
    float m = u_p1 * 3.0;
    float sphere = length(p) - 0.9;
    vec3 q = abs(p) - 0.65;
    float box = length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
    float octa = (abs(p.x) + abs(p.y) + abs(p.z) - 1.05) * 0.5773;
    float torus = length(vec2(length(p.xz) - 0.8, p.y)) - 0.28;
    if (m < 1.0) return mix(sphere, box, m);
    if (m < 2.0) return mix(box, octa, m - 1.0);
    return mix(octa, torus, m - 2.0);
}

void main() {
    vec2 uv = av_coord();
    float t = av_t();
    // Rotate the camera once (not the sample point) - a clean orbit.
    mat3 cam = rot_y(t * 0.5) * rot_x(t * 0.35);
    vec3 ro = cam * vec3(0.0, 0.0, 3.4);
    vec3 rd = cam * normalize(vec3(uv, -2.0));

    float dist = 0.0;
    float hit = 0.0;
    vec3 p = ro;
    for (int i = 0; i < 80; i++) {
        p = ro + rd * dist;
        float d = map(p);
        if (d < 0.002) { hit = 1.0; break; }
        dist += d;
        if (dist > 8.0) break;
    }

    vec3 col = vec3(0.0);
    if (hit > 0.5) {
        // Normal by gradient, for a fresnel rim.
        vec2 e = vec2(0.004, 0.0);
        vec3 n = normalize(vec3(
            map(p + e.xyy) - map(p - e.xyy),
            map(p + e.yxy) - map(p - e.yxy),
            map(p + e.yyx) - map(p - e.yyx)));
        // Wireframe: ink the edges of a lattice on the surface point.
        vec3 g = abs(fract(p * (3.0 + u_p2 * 12.0)) - 0.5);
        float line = 1.0 - smoothstep(0.0, 0.05, min(g.x, min(g.y, g.z)));
        float rim = pow(1.0 - abs(dot(n, rd)), 2.0);
        col = av_hsv(u_hue + dist * 0.08, 0.65, 1.0) * (line * 0.9 + rim * 0.7 + u_audio.x * 0.5);
    }
    FRAG_COLOR = vec4(col, 1.0);
}
