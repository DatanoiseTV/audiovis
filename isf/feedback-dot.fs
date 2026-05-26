/*{
  "DESCRIPTION": "Multi-pass feedback trails test (ISF, persistent buffer)",
  "CREDIT": "audiovis",
  "CATEGORIES": ["Generator", "Feedback"],
  "INPUTS": [
    { "NAME": "decay", "TYPE": "float", "DEFAULT": 0.95, "MIN": 0.80, "MAX": 0.995 }
  ],
  "PASSES": [
    { "TARGET": "buf", "PERSISTENT": true },
    { }
  ]
}*/
void main() {
    vec2 uv = isf_FragNormCoord;
    if (PASSINDEX == 0) {
        // Accumulate a moving dot with decay into the persistent buffer.
        vec2 c = vec2(0.5 + 0.35 * sin(TIME), 0.5 + 0.35 * cos(TIME * 1.3));
        float d = smoothstep(0.05, 0.0, distance(uv, c));
        vec3 prev = IMG_NORM_PIXEL(buf, uv).rgb * decay;
        vec3 col = max(prev, vec3(d) * vec3(0.3, 0.9, 1.0));
        gl_FragColor = vec4(col, 1.0);
    } else {
        // Present the accumulated buffer.
        gl_FragColor = IMG_NORM_PIXEL(buf, uv);
    }
}
