// A standing-in plasma used to prove the pipeline end to end before the real
// generator library lands. Pure function of UV and time, no texture reads, so
// it runs anywhere GLES2 runs.
VARYING vec2 v_uv;

uniform float u_time;
uniform vec2 u_res;
uniform float u_brightness;

void main() {
    vec2 p = v_uv * 6.2831853;
    float v = sin(p.x + u_time)
            + sin(p.y * 1.3 + u_time * 0.7)
            + sin((p.x + p.y) * 0.7 + u_time * 1.3);
    v += sin(length(v_uv - 0.5) * 18.0 - u_time * 2.0);

    vec3 col = 0.5 + 0.5 * cos(vec3(0.0, 2.094, 4.188) + v + u_time * 0.2);
    FRAG_COLOR = vec4(col * u_brightness, 1.0);
}
