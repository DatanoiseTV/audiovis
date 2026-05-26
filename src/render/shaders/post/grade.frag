// Modern colour grade: a duotone gradient-map over luminance with a contrast
// push. Blends from the original (mix 0) to a fully graded look (mix 1).
// (v_uv and u_hue come from the prepended lib; av_hsv too.)
uniform sampler2D u_tex;
uniform float u_mix;       // 0 = original, 1 = full duotone
uniform float u_contrast;

void main() {
    vec3 src = TEX2D(u_tex, v_uv).rgb;
    float l = dot(src, vec3(0.299, 0.587, 0.114));
    l = clamp((l - 0.5) * (1.0 + u_contrast * 1.6) + 0.5, 0.0, 1.0);
    vec3 duo = mix(av_hsv(u_hue, 0.7, 1.0), av_hsv(u_hue + 0.12, 0.25, 1.0), l) * (0.25 + l);
    FRAG_COLOR = vec4(mix(src, duo, u_mix), 1.0);
}
