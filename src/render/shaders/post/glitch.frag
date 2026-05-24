// Digital glitch / datamosh-flavoured corruption (Aphex Twin territory):
// horizontal block displacement, RGB channel tearing, bitcrush/posterize and
// block dropout. Bursts are gated by a time hash and the onset pulse, so it
// stutters on the beat. Prepended with lib.glsl (u_time/u_res/u_beat/u_audio,
// hashes).
uniform sampler2D u_tex;
uniform float u_intensity; // overall amount
uniform float u_blocks;    // number of horizontal tear bands
uniform float u_shift;     // RGB channel tearing
uniform float u_crush;     // bit depth reduction
uniform float u_rate;      // how often bursts fire

vec3 posterize(vec3 c, float levels) {
    return floor(c * levels + 0.5) / levels;
}

void main() {
    vec2 uv = v_uv;

    // Burst gate: random time buckets plus the audio onset.
    float bucket = floor(u_time * 12.0);
    float fire = step(1.0 - clamp(u_rate, 0.0, 1.0), av_hash(vec2(bucket, 7.0)));
    float amt = u_intensity * (0.25 + 0.75 * max(fire, u_beat));

    // Horizontal block displacement: shove whole bands sideways.
    float bands = 4.0 + floor(u_blocks * 40.0);
    float band = floor(uv.y * bands);
    float roll = av_hash(vec2(band, bucket)) - 0.5;
    // Only some bands tear, for that broken-tape randomness.
    float tear = step(0.6, av_hash(vec2(band, bucket + 1.0)));
    uv.x = fract(uv.x + roll * amt * tear * 0.5);

    // RGB tearing.
    float s = u_shift * amt * 0.05;
    vec3 col;
    col.r = TEX2D(u_tex, uv + vec2(s, 0.0)).r;
    col.g = TEX2D(u_tex, uv).g;
    col.b = TEX2D(u_tex, uv - vec2(s, 0.0)).b;

    // Block dropout: replace occasional cells with crushed/inverted data.
    vec2 cell = floor(uv * vec2(bands, bands));
    float drop = step(0.97 - amt * 0.1, av_hash(cell + bucket * 1.7));
    col = mix(col, 1.0 - col.bgr, drop * amt);

    // Bitcrush toward chunky digital steps.
    float levels = mix(255.0, 4.0, clamp(u_crush, 0.0, 1.0) * amt);
    col = posterize(col, levels);

    FRAG_COLOR = vec4(col, 1.0);
}
