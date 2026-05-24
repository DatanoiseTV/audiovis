// Shared helper functions prepended to every generator fragment shader. Kept to
// the GLES2 instruction budget: cheap hash-based noise, a 4-octave fBm, HSV and
// 2D rotation. No texture lookups, so generators run on the weakest boards.

#define PI 6.28318530718
#define HALF_PI 1.57079632679

// Uniforms every generator receives. Generators interpret the generic knobs
// (speed/scale/warp/hue/p1/p2) however suits them, so one control surface drives
// the whole library.
uniform float u_time;
uniform vec2  u_res;
uniform float u_speed;
uniform float u_scale;
uniform float u_warp;
uniform float u_hue;
uniform float u_p1;
uniform float u_p2;
uniform vec3  u_audio; // low / mid / high band energy, 0..1 (filled by audio engine)
uniform float u_beat;  // onset pulse, spikes ~1.0 on a hit and decays

VARYING vec2 v_uv;

// Aspect-corrected centered coordinate, roughly -1..1 on the short axis.
vec2 av_coord() {
    vec2 p = v_uv * 2.0 - 1.0;
    p.x *= u_res.x / max(u_res.y, 1.0);
    return p;
}

mat2 av_rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, -s, s, c);
}

float av_hash(vec2 p) {
    p = fract(p * vec2(123.34, 345.45));
    p += dot(p, p + 34.345);
    return fract(p.x * p.y);
}

// Value noise with smooth interpolation.
float av_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = av_hash(i);
    float b = av_hash(i + vec2(1.0, 0.0));
    float c = av_hash(i + vec2(0.0, 1.0));
    float d = av_hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

float av_fbm(vec2 p) {
    float v = 0.0;
    float amp = 0.5;
    for (int i = 0; i < 4; i++) {
        v += amp * av_noise(p);
        p *= 2.02;
        amp *= 0.5;
    }
    return v;
}

vec3 av_hsv(float h, float s, float v) {
    vec3 k = vec3(1.0, 2.0 / 3.0, 1.0 / 3.0);
    vec3 p = abs(fract(vec3(h) + k) * 6.0 - 3.0);
    return v * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), s);
}

// Convenience: animated time scaled by the layer speed knob.
float av_t() {
    return u_time * u_speed;
}
