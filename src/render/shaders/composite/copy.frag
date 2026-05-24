// Final present pass: copies the accumulator to the output with a master
// brightness. The analog/glitch post chain inserts ahead of this later.
VARYING vec2 v_uv;

uniform sampler2D u_tex;
uniform float u_brightness;

void main() {
    vec3 c = TEX2D(u_tex, v_uv).rgb;
    FRAG_COLOR = vec4(c * u_brightness, 1.0);
}
