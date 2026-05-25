// Audio meter bars drawn into the 2D pixel buffer.
// Set a layer's Generator to "script" to see it.
clear(0.02, 0.02, 0.05);
var bands = [low, mid, high, rms];
var n = bands.length;
var bw = Math.floor(SW / n);
for (var i = 0; i < n; i++) {
  var h = Math.floor(bands[i] * (SH - 4));
  rect(i * bw + 2, SH - h, bw - 4, h, 0.2 + 0.8 * bands[i], 0.5, 1.0 - bands[i]);
}
