// Lo-fi: pixelate (mosaic) + colour posterize, for chunky retro-digital looks.
uniform sampler2D u_tex;
uniform float u_pixels; // 0 = fine, 1 = blocky
uniform float u_levels; // 0 = full depth, 1 = few levels

void main() {
    float px = mix(320.0, 10.0, clamp(u_pixels, 0.0, 1.0));
    vec2 uv = (floor(v_uv * px) + 0.5) / px;
    vec3 c = TEX2D(u_tex, uv).rgb;
    float lv = mix(255.0, 3.0, clamp(u_levels, 0.0, 1.0));
    c = floor(c * lv + 0.5) / lv;
    FRAG_COLOR = vec4(c, 1.0);
}
