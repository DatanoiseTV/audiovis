// Lettering overlay with selectable font (bound on the CPU side) and text FX.
// fx: 0 none, 1 pixel-dissolve (pops in with the fade), 2 wave/warp,
// 3 row tear, 4 scanlines. Prepended with lib.glsl.
uniform sampler2D u_font;  // 16x16 glyph atlas (alpha = ink)
uniform sampler2D u_text;  // 1 x N code texture
uniform vec2 u_start;      // top-left of the text block, in uv
uniform vec2 u_glyph;      // glyph cell size in uv (w, h)
uniform float u_count;
uniform float u_alpha;     // fade 0..1
uniform float u_fxamt;
uniform int u_fx;

void main() {
    vec2 uv = v_uv;

    // Wave / warp distorts the lookup coordinates.
    if (u_fx == 2) {
        uv.y += sin(uv.x * 24.0 + u_time * 3.0) * u_fxamt * 0.04;
        uv.x += sin(uv.y * 18.0 - u_time * 2.0) * u_fxamt * 0.02;
    }

    float rowf = (u_start.y - uv.y) / u_glyph.y;
    if (rowf < 0.0 || rowf >= 1.0) discard;

    // Row tear.
    float jit = av_hash(vec2(floor(rowf * 16.0), floor(u_time * 18.0))) - 0.5;
    float tear = u_fx == 3 ? jit * u_fxamt * 0.5 : 0.0;
    float colf = (uv.x - u_start.x) / u_glyph.x + tear;
    if (colf < 0.0 || colf >= u_count) discard;

    float code = floor(TEX2D(u_text, vec2((floor(colf) + 0.5) / u_count, 0.5)).r * 255.0 + 0.5);
    vec2 g = vec2(fract(colf), rowf);
    vec2 atlas = (vec2(mod(code, 16.0), floor(code / 16.0)) + g) / 16.0;
    if (TEX2D(u_font, atlas).a < 0.5) discard;

    float a = u_alpha;
    // Pixel dissolve: each pixel crosses its random threshold as the fade rises.
    if (u_fx == 1) {
        if (av_hash(floor(v_uv * u_res * 0.5)) > u_alpha) discard;
        a = 1.0;
    }
    if (u_fx == 4) {
        a *= step(0.5, fract(v_uv.y * u_res.y / 3.0));
    }

    vec3 col = av_hsv(u_hue, 0.55, 1.0);
    FRAG_COLOR = vec4(col, a);
}
