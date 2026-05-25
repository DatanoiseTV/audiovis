// Orbiting particles that bloom with the audio - light and fast.
// Set a layer's Generator to "script" to see it.
clear(0.01, 0.01, 0.04);
var n = 48;
for (var i = 0; i < n; i++) {
  var a = i / n * 6.2832;
  var rad = 18 + 12 * Math.sin(t * 0.7 + i * 0.3) + low * 34;
  var x = SW / 2 + Math.cos(a + t * 0.5) * rad * 1.7;
  var y = SH / 2 + Math.sin(a + t * 0.5) * rad;
  var s = 1 + Math.floor(high * 3);
  rect(x - s, y - s, s * 2, s * 2, 0.5 + 0.5 * Math.sin(a * 2 + t), 0.4, 1.0 - i / n);
}
