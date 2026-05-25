// Digital glitch / datamosh-flavoured corruption (Aphex Twin territory),
// layered: multi-scale slice displacement, RGB channel desync, DCT/JPEG-style
// block quantization, scanline dropouts and bitcrush. Bursts are gated by a
// time hash and the onset/highs so it stutters with the music. Prepended with
// lib.glsl (u_time/u_res/u_beat/u_audio, hashes).
uniform sampler2D u_tex;
uniform float u_intensity; // master amount
uniform float u_blocks;    // slice displacement density
uniform float u_shift;     // RGB channel tearing
uniform float u_crush;     // block quantization + bit depth
uniform float u_rate;      // burst frequency

vec3 rgb_split(vec2 uv, float s) {
    return vec3(TEX2D(u_tex, uv + vec2(s, 0.0)).r, TEX2D(u_tex, uv).g, TEX2D(u_tex, uv - vec2(s, 0.0)).b);
}

void main() {
    float bucket = floor(u_time * 12.0);
    float fire = step(1.0 - clamp(u_rate, 0.0, 1.0), av_hash(vec2(bucket, 7.0)));
    float amt = clamp(u_intensity * (0.3 + 0.7 * max(fire, u_beat + u_audio.z)), 0.0, 1.0);

    vec2 uv = v_uv;

    // 1. Two-scale horizontal slice displacement (big bands + fine lines).
    for (int k = 0; k < 2; k++) {
        float bands = (4.0 + floor(u_blocks * 30.0)) * (k == 0 ? 1.0 : 4.0);
        float band = floor(uv.y * bands);
        float h = av_hash(vec2(band, bucket + float(k) * 5.0));
        float tear = step(0.55, av_hash(vec2(band, bucket + float(k) * 9.0)));
        uv.x = fract(uv.x + (h - 0.5) * amt * 0.5 * tear);
    }

    // 2. RGB channel desync, jittered per scanline.
    float s = u_shift * amt * 0.05 * (0.5 + av_hash(vec2(floor(uv.y * 90.0), bucket)));
    vec3 col = rgb_split(uv, s);

    // 3. DCT/JPEG-style block quantization: snap to blocks + posterize them.
    float bs = mix(2.0, 28.0, u_crush);
    vec2 buv = (floor(uv * u_res.xy / bs) * bs + bs * 0.5) / u_res.xy;
    vec3 blockc = rgb_split(buv, s);
    blockc = floor(blockc * 6.0 + 0.5) / 6.0; // coarse per-block colour
    col = mix(col, blockc, u_crush * amt);

    // 4. Scanline dropouts: a few rows go to inverted/over-bright garbage.
    float drop = step(0.988, av_hash(vec2(floor(uv.y * u_res.y / 2.0), bucket * 3.0)));
    col = mix(col, (1.0 - col.gbr) * 1.4, drop * amt);

    // 5. Global bitcrush.
    float levels = mix(255.0, 5.0, u_crush * amt);
    col = floor(col * levels + 0.5) / levels;

    FRAG_COLOR = vec4(col, 1.0);
}
