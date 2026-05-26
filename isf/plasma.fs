/*{
  "DESCRIPTION": "Simple audiovis test plasma (ISF)",
  "CREDIT": "audiovis",
  "CATEGORIES": ["Generator"],
  "INPUTS": [
    { "NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0 },
    { "NAME": "scale", "TYPE": "float", "DEFAULT": 6.0, "MIN": 1.0, "MAX": 24.0 },
    { "NAME": "tint", "TYPE": "color", "DEFAULT": [0.0, 0.5, 1.0, 1.0] }
  ]
}*/
void main() {
    vec2 uv = isf_FragNormCoord;
    float t = TIME * speed;
    float v = sin(uv.x * scale + t)
            + sin(uv.y * scale + t * 1.3)
            + sin((uv.x + uv.y) * scale * 0.7 + t * 0.7);
    vec3 col = 0.5 + 0.5 * cos(vec3(0.0, 2.0, 4.0) + v + t);
    col = mix(col, tint.rgb, 0.35);
    gl_FragColor = vec4(col, 1.0);
}
