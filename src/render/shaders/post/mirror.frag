// Mirror / kaleidoscope of the whole frame.
//   0 mirror X, 1 mirror Y, 2 quad, 3 kaleidoscope
uniform sampler2D u_tex;
uniform int u_mode;

void main() {
    vec2 uv = v_uv;
    if (u_mode == 1) {
        if (uv.y > 0.5) uv.y = 1.0 - uv.y;
    } else if (u_mode == 2) {
        if (uv.x > 0.5) uv.x = 1.0 - uv.x;
        if (uv.y > 0.5) uv.y = 1.0 - uv.y;
    } else if (u_mode == 3) {
        vec2 p = uv - 0.5;
        float a = atan(p.y, p.x);
        float r = length(p);
        float seg = PI / 3.0;
        a = mod(a, seg);
        a = abs(a - seg * 0.5);
        uv = 0.5 + r * vec2(cos(a), sin(a));
    } else {
        if (uv.x > 0.5) uv.x = 1.0 - uv.x;
    }
    FRAG_COLOR = vec4(TEX2D(u_tex, uv).rgb, 1.0);
}
