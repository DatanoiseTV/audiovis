// Procedural fire rising from the bottom edge, brightened by the bass.
void main() {
    vec2 uv = v_uv;
    float t = av_t();
    float n = av_fbm(vec2(uv.x * 5.0 * u_scale, uv.y * 3.0 - t * 2.0));
    n += 0.5 * av_fbm(vec2(uv.x * 11.0, uv.y * 6.0 - t * 3.5));
    // Subtract height so flames taper upward; bass lifts them.
    float fire = n * (1.0 + u_audio.x * 2.0) - uv.y * 1.5;
    fire = clamp(fire, 0.0, 1.0);
    vec3 col = vec3(fire * 1.6, fire * fire * 0.9, fire * fire * fire * 0.4);
    FRAG_COLOR = vec4(col, 1.0);
}
