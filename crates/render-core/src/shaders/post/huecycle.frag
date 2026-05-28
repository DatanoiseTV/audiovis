// Hue rotation / colour cycling. Rotates colour around the (1,1,1) axis
// (luminance-preserving), with an optional auto-cycle. lib.glsl provides u_time.
uniform sampler2D u_tex;
uniform float u_shift; // static hue offset 0..1
uniform float u_rate;  // auto-cycle speed

vec3 hueshift(vec3 c, float a) {
    vec3 k = vec3(0.57735027);
    float ca = cos(a);
    return c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca);
}

void main() {
    vec3 c = TEX2D(u_tex, v_uv).rgb;
    float a = (u_shift + u_time * u_rate) * 6.28318530718;
    FRAG_COLOR = vec4(clamp(hueshift(c, a), 0.0, 1.0), 1.0);
}
