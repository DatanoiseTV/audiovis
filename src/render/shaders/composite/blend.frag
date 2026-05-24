// Blends one layer over the running accumulator using a selectable mode and
// opacity. Driven once per layer by the compositor.
VARYING vec2 v_uv;

uniform sampler2D u_base;   // accumulator so far
uniform sampler2D u_top;    // this layer
uniform float u_opacity;
uniform int u_mode;         // 0 normal, 1 add, 2 screen, 3 multiply, 4 difference

void main() {
    vec3 b = TEX2D(u_base, v_uv).rgb;
    vec3 t = TEX2D(u_top, v_uv).rgb;

    vec3 r;
    if (u_mode == 1) {
        r = b + t;
    } else if (u_mode == 2) {
        r = 1.0 - (1.0 - b) * (1.0 - t);
    } else if (u_mode == 3) {
        r = b * t;
    } else if (u_mode == 4) {
        r = abs(b - t);
    } else {
        r = t;
    }

    FRAG_COLOR = vec4(mix(b, r, clamp(u_opacity, 0.0, 1.0)), 1.0);
}
