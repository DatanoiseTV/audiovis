// Composites a loaded image/SVG layer over the accumulator in a single pass.
// The media texture is sampled through a zoom/rotate/pan transform with its own
// aspect ratio preserved, recoloured (hue rotate + brightness), then blended
// over the running accumulator using the same mode set as the generator layers.
VARYING vec2 v_uv;

uniform sampler2D u_base;   // accumulator so far
uniform sampler2D u_tex;    // loaded media (straight alpha)
uniform vec2 u_res;         // layer resolution, for output aspect
uniform float u_aspect;     // media width / height
uniform float u_zoom;
uniform float u_rot;        // radians
uniform vec2 u_pan;
uniform float u_hue;        // radians of hue rotation
uniform float u_bright;
uniform float u_opacity;
uniform int u_mode;         // matches blend.frag: 0 normal 1 add 2 screen 3 mul 4 diff

// Rotate an RGB colour about the (1,1,1) grey axis - a clean hue shift.
vec3 hue_rotate(vec3 col, float a) {
    const vec3 k = vec3(0.57735026);
    float c = cos(a);
    float s = sin(a);
    return col * c + cross(k, col) * s + k * dot(k, col) * (1.0 - c);
}

void main() {
    // Map the fragment into a centred, output-aspect-correct space.
    vec2 uv = v_uv - 0.5;
    uv.x *= u_res.x / max(u_res.y, 1.0);

    // Apply the layer transform: pan, then rotate, then zoom.
    uv -= u_pan;
    float si = sin(u_rot);
    float co = cos(u_rot);
    uv = mat2(co, -si, si, co) * uv;
    uv /= max(u_zoom, 0.0001);

    // Into media space: preserve the image's own aspect, flip Y to upright.
    uv.x /= max(u_aspect, 0.0001);
    vec2 iuv = vec2(uv.x, -uv.y) + 0.5;

    vec4 img = TEX2D(u_tex, iuv);
    float inside = step(0.0, iuv.x) * step(iuv.x, 1.0) * step(0.0, iuv.y) * step(iuv.y, 1.0);
    img.a *= inside;
    img.rgb = hue_rotate(img.rgb, u_hue) * u_bright;

    vec3 base = TEX2D(u_base, v_uv).rgb;
    vec3 r;
    if (u_mode == 1) {
        r = base + img.rgb;
    } else if (u_mode == 2) {
        r = 1.0 - (1.0 - base) * (1.0 - img.rgb);
    } else if (u_mode == 3) {
        r = base * img.rgb;
    } else if (u_mode == 4) {
        r = abs(base - img.rgb);
    } else {
        r = img.rgb;
    }

    float a = clamp(u_opacity * img.a, 0.0, 1.0);
    FRAG_COLOR = vec4(mix(base, r, a), 1.0);
}
