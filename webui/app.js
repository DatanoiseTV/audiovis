// audiovis web control surface.
//
// Loads the shared control.proto at runtime (single source of truth with the
// Rust server), opens a binary websocket, builds the control UI from the
// parameter schema the server sends, and keeps everything in two-way sync.

"use strict";

// Bump on every UI change so it is obvious in the console whether the browser
// is running fresh assets or a stale cached copy.
const UI_BUILD = "ui-14";
console.log(`audiovis ${UI_BUILD} loaded`);

const BLEND_NAMES = ["normal", "add", "screen", "multiply", "difference"];
const DIV_NAMES = ["8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16"];
const SHAPE_NAMES = ["sine", "triangle", "saw up", "saw dn", "square", "pulse", "rand", "noise", "steps"];
const FONT_NAMES = ["system", "bold", "outline", "alien"];
const TEXTFX_NAMES = ["none", "dissolve", "wave", "tear", "scanlines"];

// Named option lists for integer params that read better as dropdowns.
function intOptions(path) {
  if (path.endsWith(".generator")) return generators;
  if (path.endsWith(".blend")) return BLEND_NAMES;
  if (path.endsWith(".div")) return DIV_NAMES;
  if (path.endsWith(".shape")) return SHAPE_NAMES;
  if (path === "text.font") return FONT_NAMES;
  if (path === "text.fx") return TEXTFX_NAMES;
  return null;
}

let ServerMsg, ClientMsg;
let ws = null;
let armed = null; // path currently in MIDI/OSC learn
const widgets = new Map(); // path -> { set(value, norm), spec }
const specsByPath = new Map();
const paramValues = new Map(); // path -> { value, norm }, latest known
let generators = [];
let modSources = [];
let latestTelemetry = null;
let blackoutPrev = null; // brightness remembered while blacked out
let presetList = [];
let currentPreset = "";
const textSlots = [];
const textInputs = new Map(); // slot -> input element

// LFO division lengths in beats (must match Rust LFO_DIVISIONS).
const DIV_BEATS = [32, 16, 8, 4, 2, 1, 0.5, 0.25];

async function main() {
  // protobuf.min.js is a plain <script> before this one, so the global should
  // exist; retry briefly in case of odd load ordering before giving up.
  for (let i = 0; i < 20 && typeof protobuf === "undefined"; i++) {
    await new Promise((r) => setTimeout(r, 50));
  }
  if (typeof protobuf === "undefined") {
    return fail("protobuf.js failed to load");
  }
  try {
    setConn("loading schema");
    // Parse the schema kept in sync with the server. keepCase so JS field names
    // match the .proto exactly (is_norm, beat_phase, ...).
    const res = await fetch("control.proto", { cache: "no-store" });
    if (!res.ok) return fail(`control.proto ${res.status}`);
    const root = protobuf.parse(await res.text(), { keepCase: true }).root;
    ServerMsg = root.lookupType("audiovis.ServerMsg");
    ClientMsg = root.lookupType("audiovis.ClientMsg");
  } catch (e) {
    console.error(e);
    return fail("schema: " + (e && e.message ? e.message : e));
  }
  setupTransport();
  requestAnimationFrame(drawLfos);
  connect();
}

// Master fader, blackout and the spacebar kill - the controls a VJ reaches for.
function setupTransport() {
  const master = document.getElementById("master");
  master.oninput = () => sendNorm("global.brightness", parseFloat(master.value));

  const bo = document.getElementById("blackout");
  bo.onclick = () => toggleBlackout();
  document.addEventListener("keydown", (e) => {
    if (e.code === "Space" && e.target.tagName !== "INPUT") { e.preventDefault(); toggleBlackout(); }
  });
}

function toggleBlackout() {
  const bo = document.getElementById("blackout");
  if (blackoutPrev === null) {
    blackoutPrev = paramValues.get("global.brightness")?.norm ?? 1;
    sendNorm("global.brightness", 0);
    bo.classList.add("active");
  } else {
    sendNorm("global.brightness", blackoutPrev);
    blackoutPrev = null;
    bo.classList.remove("active");
  }
}

function setConn(text) {
  const el = document.getElementById("conn");
  if (el) el.textContent = text;
}

function fail(msg) {
  console.error("audiovis ui:", msg);
  setConn(msg);
}

function connect() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  setConn("connecting");
  try {
    ws = new WebSocket(`${proto}://${location.host}/ws`);
  } catch (e) {
    return fail("ws: " + (e && e.message ? e.message : e));
  }
  ws.binaryType = "arraybuffer";

  ws.onopen = () => setStatus(true);
  ws.onerror = () => setStatus(false);
  ws.onclose = () => {
    setStatus(false);
    setTimeout(connect, 1000); // auto-reconnect
  };
  ws.onmessage = (ev) => {
    try {
      const msg = ServerMsg.decode(new Uint8Array(ev.data));
      if (msg.schema && msg.schema.length) {
        generators = msg.generators || [];
        if (msg.mod_sources && msg.mod_sources.length) modSources = msg.mod_sources;
        buildUI(msg.schema);
      }
      if (msg.changes) for (const c of msg.changes) applyChange(c);
      if (msg.telemetry) applyTelemetry(msg.telemetry);
      if (msg.mod_routes_present) renderRoutes(msg.mod_routes || []);
      if (msg.current_preset) currentPreset = msg.current_preset;
      if (msg.presets && msg.presets.length) { presetList = msg.presets; renderPresets(); }
      else if (msg.current_preset) renderPresets();
      if (msg.text && msg.text.length) { for (const t of msg.text) textSlots[t.id] = t.text; refreshTextInputs(); }
    } catch (e) {
      console.error("decode error", e);
    }
  };
}

function setStatus(up) {
  const dot = document.getElementById("dot");
  if (dot) dot.classList.toggle("up", up);
  setConn(up ? "live" : "reconnecting");
}

// --- sending ---

function send(obj) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(ClientMsg.encode(ClientMsg.create(obj)).finish());
  }
}
const sendNorm = (path, norm) => send({ set: { path, is_norm: true, value: norm } });
const sendRaw = (path, value) => send({ set: { path, is_norm: false, value } });
const sendTrigger = (path) => send({ set: { path, trigger: true } });
const sendLearn = (path, arm, clear) => send({ learn: { path, arm, clear } });
const sendPreset = (action, path) => send({ preset: { action, path } });
const sendText = (id, text) => send({ text: { id, text } });

// --- UI building ---

const FX_GROUPS = ["Feedback", "Mirror", "Hue cycle", "Lo-fi", "VHS", "Glitch", "Bloom"];

// Order groups for a live surface: layers first, then the effects rack, then
// LFOs / clock / global.
function groupOrder(name) {
  if (name.startsWith("Layer")) return 0 + (parseInt(name.replace(/\D/g, ""), 10) || 0);
  const fx = FX_GROUPS.indexOf(name);
  if (fx >= 0) return 10 + fx;
  return { LFO: 30, Clock: 31, Global: 32 }[name] ?? 40;
}

function buildUI(schema) {
  for (const s of schema) specsByPath.set(s.path, s);
  widgets.clear();
  const main = document.getElementById("groups");
  main.innerHTML = "";

  const groups = new Map();
  for (const s of schema) {
    if (!groups.has(s.group)) groups.set(s.group, []);
    groups.get(s.group).push(s);
  }

  const names = [...groups.keys()].sort((a, b) => groupOrder(a) - groupOrder(b));
  for (const name of names) {
    if (name === "Text") { main.appendChild(buildLettering(groups.get(name))); continue; }
    const cls = name.startsWith("Layer") ? "group layer" : FX_GROUPS.includes(name) ? "group fx" : "group";
    const sec = el("div", cls);
    sec.appendChild(el("h2", null, name));
    for (const spec of groups.get(name)) sec.appendChild(buildRow(spec));
    if (name === "LFO") sec.appendChild(buildLfoScopes());
    main.appendChild(sec);
  }
  main.appendChild(buildMatrix());
  main.appendChild(buildPresets());
}

// --- live LFO shape previews ---

function buildLfoScopes() {
  const wrap = el("div", "lfoscopes");
  for (let n = 1; n <= 6; n++) {
    const box = el("div", "lfobox");
    const cv = el("canvas"); cv.width = 150; cv.height = 46; cv.id = `lfoscope-${n}`;
    box.append(el("span", "lfol", `lfo ${n}`), cv);
    wrap.appendChild(box);
  }
  return wrap;
}

// One LFO sample, matching the Rust lfo() (sine, tri, saw up/dn, square, pulse,
// rand, smooth noise, steps).
function cycleRand(c) { const v = Math.sin(c * 12.9898) * 43758.547; return (v - Math.floor(v)) * 2 - 1; }
function lfoValue(shape, phase) {
  const f = ((phase % 1) + 1) % 1;
  switch (shape) {
    case 1: return 4 * Math.abs(f - 0.5) - 1;
    case 2: return 2 * f - 1;
    case 3: return 1 - 2 * f;
    case 4: return f < 0.5 ? 1 : -1;
    case 5: return f < 0.25 ? 1 : -1;
    case 6: return cycleRand(Math.floor(phase));
    case 7: { const c = Math.floor(phase), t = f * f * (3 - 2 * f); return cycleRand(c) * (1 - t) + cycleRand(c + 1) * t; }
    case 8: return Math.floor(f * 8) / 3.5 - 1;
    default: return Math.sin(6.28318530718 * f);
  }
}

function drawLfos() {
  requestAnimationFrame(drawLfos);
  const beats = latestTelemetry ? latestTelemetry.beats || 0 : 0;
  for (let n = 1; n <= 6; n++) {
    const cv = document.getElementById(`lfoscope-${n}`);
    if (!cv) continue;
    const ctx = cv.getContext("2d");
    const w = cv.width, h = cv.height, mid = h / 2;
    const div = DIV_BEATS[Math.round(paramValues.get(`lfo.${n}.div`)?.value ?? 3)] || 4;
    const shape = Math.round(paramValues.get(`lfo.${n}.shape`)?.value ?? 0);
    const phase = (beats / div) % 1;

    ctx.clearRect(0, 0, w, h);
    ctx.strokeStyle = "#262633"; ctx.beginPath(); ctx.moveTo(0, mid); ctx.lineTo(w, mid); ctx.stroke();
    // waveform over one cycle
    ctx.strokeStyle = "#35e0d8"; ctx.lineWidth = 1.5; ctx.beginPath();
    for (let x = 0; x <= w; x++) {
      const y = mid - lfoValue(shape, x / w) * (mid - 3);
      x === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
    }
    ctx.stroke();
    // playhead + live value dot
    const px = phase * w;
    ctx.strokeStyle = "#ff3ea5"; ctx.beginPath(); ctx.moveTo(px, 0); ctx.lineTo(px, h); ctx.stroke();
    const vy = mid - lfoValue(shape, phase) * (mid - 3);
    ctx.fillStyle = "#ff3ea5"; ctx.beginPath(); ctx.arc(px, vy, 2.5, 0, 6.2832); ctx.fill();
  }
}

// --- modulation matrix: grid patchbay ---
//
// Columns are sources, rows are target params. A cell holds the route amount;
// drag it vertically to set (bipolar), right-click to clear. Click selects a
// cell for fine amount/smooth editing below the grid. Same protocol as before
// (ModRouteCmd), just a denser view.

const sendMod = (source, target, amount, smooth) => send({ mod: { source, target, amount, smooth } });

const SRC_SHORT = {
  "audio.low": "lo", "audio.mid": "mid", "audio.high": "hi", "audio.rms": "rms",
  "audio.level": "lvl", "audio.beat": "beat", "clock.beat": "ck·b", "clock.bar": "ck·r",
  "lfo.1": "lfo1", "lfo.2": "lfo2", "lfo.3": "lfo3",
};

const routesMap = new Map(); // "source|target" -> { amount, smooth }
const gridTargets = [];      // ordered target paths shown as rows
const cellMap = new Map();   // "source|target" -> cell element
let gridEl = null, selEditorEl = null, targetPicker = null;
let selected = null;         // { source, target }
let drag = null;             // active cell drag state
let dragListenersAdded = false;

function floatTargets() {
  const out = [];
  for (const s of specsByPath.values()) if (s.kind === "float") out.push(s);
  return out;
}

function buildMatrix() {
  const sec = el("div", "group matrix");
  sec.appendChild(el("h2", null, "Modulation matrix"));

  const add = el("div", "gridadd");
  targetPicker = el("select");
  const addBtn = el("button", "btn", "+ row");
  addBtn.onclick = () => {
    const v = targetPicker.value;
    if (v && !gridTargets.includes(v)) { gridTargets.push(v); rebuildGrid(); }
  };
  add.append(el("span", "gl", "add target row:"), targetPicker, addBtn);
  sec.appendChild(add);

  gridEl = el("div", "grid");
  sec.appendChild(gridEl);
  selEditorEl = el("div", "selrow");
  sec.appendChild(selEditorEl);

  if (!dragListenersAdded) {
    document.addEventListener("pointermove", onCellDragMove);
    document.addEventListener("pointerup", onCellDragEnd);
    dragListenersAdded = true;
  }
  rebuildGrid();
  return sec;
}

// Rebuild the whole grid DOM (only when the set of rows changes).
function rebuildGrid() {
  if (!gridEl) return;
  gridEl.innerHTML = "";
  cellMap.clear();
  gridEl.style.gridTemplateColumns = `130px repeat(${modSources.length}, 1fr)`;

  // Header: corner + source columns.
  gridEl.appendChild(el("div", "corner"));
  for (const s of modSources) gridEl.appendChild(el("div", "srchead", SRC_SHORT[s] || s));

  // One row per target.
  for (const target of gridTargets) {
    const spec = specsByPath.get(target);
    const label = el("div", "rowlabel", spec ? `${spec.group}/${spec.name}` : target);
    label.title = target;
    gridEl.appendChild(label);
    for (const source of modSources) {
      const cell = makeCell(source, target);
      gridEl.appendChild(cell);
      cellMap.set(`${source}|${target}`, cell);
    }
  }

  // Repopulate the add-target picker with rows not already shown.
  targetPicker.innerHTML = "";
  floatTargets().filter((s) => !gridTargets.includes(s.path)).forEach((s) => {
    const o = el("option", null, `${s.group} / ${s.name}`); o.value = s.path; targetPicker.appendChild(o);
  });

  refreshCells();
  renderSelected();
}

function makeCell(source, target) {
  const cell = el("div", "cell");
  cell.appendChild(el("div", "fill"));
  cell.appendChild(el("span", "camt"));
  cell.onpointerdown = (e) => {
    e.preventDefault();
    const r = routesMap.get(`${source}|${target}`);
    drag = { source, target, cell, startY: e.clientY, start: r ? r.amount : 0, moved: false };
    selected = { source, target };
    renderSelected();
  };
  cell.oncontextmenu = (e) => { e.preventDefault(); sendMod(source, target, 0, 0); };
  return cell;
}

function onCellDragMove(e) {
  if (!drag) return;
  drag.moved = true;
  const amt = Math.max(-1, Math.min(1, drag.start + (drag.startY - e.clientY) * 0.006));
  setCell(drag.cell, amt);
  const prev = routesMap.get(`${drag.source}|${drag.target}`);
  sendMod(drag.source, drag.target, amt, prev ? prev.smooth : 0.0);
}

function onCellDragEnd() {
  if (!drag) return;
  // A click without movement seeds a sensible default so a tap creates a route.
  if (!drag.moved && !routesMap.has(`${drag.source}|${drag.target}`)) {
    sendMod(drag.source, drag.target, 0.5, 0.0);
  }
  drag = null;
}

// Paint a cell from an amount: bipolar fill (cyan up / magenta down) + number.
function setCell(cell, amount) {
  const fill = cell.firstChild;
  const txt = cell.lastChild;
  const a = amount || 0;
  cell.classList.toggle("on", Math.abs(a) > 0.001);
  const h = Math.min(50, Math.abs(a) * 50);
  if (a >= 0) { fill.style.bottom = "50%"; fill.style.top = ""; fill.style.background = "var(--cyan)"; }
  else { fill.style.top = "50%"; fill.style.bottom = ""; fill.style.background = "var(--magenta)"; }
  fill.style.height = `${h}%`;
  txt.textContent = Math.abs(a) > 0.001 ? a.toFixed(1) : "";
}

function refreshCells() {
  for (const [key, cell] of cellMap) {
    if (drag && drag.cell === cell) continue; // don't fight a live drag
    const r = routesMap.get(key);
    setCell(cell, r ? r.amount : 0);
  }
}

function renderSelected() {
  if (!selEditorEl) return;
  selEditorEl.innerHTML = "";
  if (!selected) { selEditorEl.appendChild(el("div", "empty", "click a cell to edit amount + smoothing")); return; }
  const key = `${selected.source}|${selected.target}`;
  const r = routesMap.get(key) || { amount: 0, smooth: 0 };
  const spec = specsByPath.get(selected.target);
  selEditorEl.appendChild(el("div", "rlabel", `${selected.source} -> ${spec ? spec.name : selected.target}`));

  const mk = (label, min, val, onin) => {
    const w = el("div", "rng");
    const i = el("input"); i.type = "range"; i.min = min; i.max = 1; i.step = 0.01; i.value = val;
    i.oninput = onin; w.append(el("span", "rngl", label), i); return { w, i };
  };
  const send = () => sendMod(selected.source, selected.target, parseFloat(amt.i.value), parseFloat(sm.i.value));
  const amt = mk("amount", -1, r.amount || 0, send);
  const sm = mk("smooth", 0, r.smooth || 0, send);
  const rm = el("button", "btn", "clear");
  rm.onclick = () => sendMod(selected.source, selected.target, 0, 0);
  selEditorEl.append(amt.w, sm.w, rm);
  selEditorEl._amt = amt.i;
  selEditorEl._sm = sm.i;
}

function renderRoutes(routes) {
  // Rebuild the route lookup and add rows for any target that gained a route.
  routesMap.clear();
  let needRebuild = false;
  for (const r of routes) {
    routesMap.set(`${r.source}|${r.target}`, { amount: r.amount || 0, smooth: r.smooth || 0 });
    if (!gridTargets.includes(r.target)) { gridTargets.push(r.target); needRebuild = true; }
  }
  if (!gridEl) return;
  if (needRebuild) { rebuildGrid(); return; }
  refreshCells();

  // Keep the selection editor's sliders in sync unless being dragged.
  if (selected && selEditorEl._amt) {
    const r = routesMap.get(`${selected.source}|${selected.target}`) || { amount: 0, smooth: 0 };
    if (document.activeElement !== selEditorEl._amt) selEditorEl._amt.value = r.amount || 0;
    if (document.activeElement !== selEditorEl._sm) selEditorEl._sm.value = r.smooth || 0;
  }
}

function buildRow(spec) {
  const row = el("div", "row");
  row.appendChild(el("div", "name", spec.name));

  const mid = el("div");
  const val = el("div", "val");
  let widget;

  if (spec.kind === "trigger") {
    const b = el("button", "btn trigger", spec.name);
    b.onclick = () => sendTrigger(spec.path);
    mid.appendChild(b);
    widget = { set: () => {} };
  } else if (spec.kind === "bool") {
    const t = el("div", "toggle");
    t.appendChild(el("div", "knob"));
    t.onclick = () => { const on = !t.classList.contains("on"); sendRaw(spec.path, on ? 1 : 0); };
    mid.appendChild(t);
    val.style.display = "none";
    widget = { set: (v) => t.classList.toggle("on", v >= 0.5) };
  } else if (spec.kind === "int" && intOptions(spec.path)) {
    const sel = el("select");
    const opts = intOptions(spec.path);
    opts.forEach((nm, i) => { const o = el("option", null, nm); o.value = i; sel.appendChild(o); });
    sel.onchange = () => sendRaw(spec.path, parseInt(sel.value, 10));
    mid.appendChild(sel);
    val.style.display = "none";
    widget = { set: (v) => { sel.value = Math.round(v); } };
  } else {
    // float or generic int: a normalised slider, value display shows raw.
    const r = el("input"); r.type = "range"; r.min = 0; r.max = 1; r.step = 0.001;
    r.oninput = () => sendNorm(spec.path, parseFloat(r.value));
    mid.appendChild(r);
    widget = {
      set: (v, norm) => {
        if (document.activeElement !== r) r.value = norm;
        val.textContent = fmt(v, spec);
      },
    };
  }
  row.appendChild(mid);

  // Learn / clear affordance for everything mappable.
  const ctl = el("div");
  if (spec.kind !== "trigger") ctl.appendChild(val);
  const learn = el("button", "btn learn", "L");
  learn.title = "MIDI/OSC learn";
  learn.onclick = () => {
    if (armed === spec.path) { sendLearn(spec.path, false, false); setArmed(null); }
    else { sendLearn(spec.path, true, false); setArmed(spec.path); }
  };
  learn.oncontextmenu = (e) => { e.preventDefault(); sendLearn(spec.path, false, true); };
  ctl.appendChild(learn);
  row.appendChild(ctl);

  widget.learnBtn = learn;
  widgets.set(spec.path, widget);
  return row;
}

// --- lettering bank panel ---

function buildLettering(specs) {
  const sec = el("div", "group");
  sec.appendChild(el("h2", null, "Lettering"));
  textInputs.clear();
  for (let n = 0; n < 8; n++) {
    const row = el("div", "textrow");
    const inp = el("input"); inp.type = "text"; inp.placeholder = `slot ${n + 1}`; inp.value = textSlots[n] || "";
    inp.oninput = () => sendText(n, inp.value);
    const show = el("button", "btn", "show"); show.onclick = () => sendTrigger(`text.${n}.trigger`);
    row.append(inp, show);
    sec.appendChild(row);
    textInputs.set(n, inp);
  }
  const clr = el("button", "btn", "clear"); clr.onclick = () => sendTrigger("text.clear");
  sec.appendChild(clr);
  // Style params (font / fx / size / pos / hue); the per-slot triggers are the
  // "show" buttons above, so skip trigger kinds here.
  for (const spec of specs) {
    if (spec.kind === "trigger") continue;
    sec.appendChild(buildRow(spec));
  }
  return sec;
}

function refreshTextInputs() {
  for (const [n, inp] of textInputs) {
    if (document.activeElement !== inp) inp.value = textSlots[n] || "";
  }
}

let presetListEl = null;

function buildPresets() {
  const sec = el("div", "group matrix");
  sec.appendChild(el("h2", null, "Presets"));
  presetListEl = el("div", "presetlist");
  sec.appendChild(presetListEl);

  const bar = el("div", "preset-bar");
  const input = el("input"); input.type = "text"; input.placeholder = "name to save"; input.id = "preset-name";
  const save = el("button", "btn", "save as");
  save.onclick = () => { if (input.value.trim()) sendPreset("save", input.value.trim()); };
  bar.append(input, save);
  sec.appendChild(bar);
  renderPresets();
  return sec;
}

function renderPresets() {
  if (!presetListEl) return;
  presetListEl.innerHTML = "";
  for (const name of presetList) {
    const b = el("button", "btn preset" + (name === currentPreset ? " active" : ""), name);
    b.onclick = () => sendPreset("load", name);
    presetListEl.appendChild(b);
  }
}

function setArmed(path) {
  armed = path;
  for (const w of widgets.values()) if (w.learnBtn) w.learnBtn.classList.toggle("armed", w === widgets.get(path));
}

// --- incoming state ---

function applyChange(c) {
  paramValues.set(c.path, { value: c.value || 0, norm: c.norm || 0 });
  const w = widgets.get(c.path);
  if (w) w.set(c.value || 0, c.norm || 0);
  // Keep the header master fader in sync (it is not a registered widget).
  if (c.path === "global.brightness") {
    const m = document.getElementById("master");
    if (m && document.activeElement !== m) m.value = c.norm || 0;
  }
}

function applyTelemetry(t) {
  latestTelemetry = t;
  setMeter("m-low", t.low); setMeter("m-mid", t.mid); setMeter("m-high", t.high); setMeter("m-rms", t.rms);
  const beat = document.getElementById("beat");
  // Flash on the audio onset or on each clock downbeat, so it visibly ticks.
  const onDownbeat = (t.beat_phase || 0) < 0.12;
  beat.classList.toggle("hit", (t.beat || 0) > 0.5 || onDownbeat);
  document.getElementById("bpm").textContent = `${Math.round(t.bpm || 0)} BPM`;
  const bf = document.getElementById("beatfill");
  if (bf) bf.style.width = `${(t.beat_phase || 0) * 100}%`;

  // The clock phase/tempo are telemetry, not notified params (they change every
  // frame), so drive the Clock-group widgets directly from telemetry here.
  driveWidget("clock.bpm", t.bpm || 0);
  driveWidget("clock.beat", t.beat_phase || 0);
  driveWidget("clock.bar", t.bar_phase || 0);
}

// Update a widget from a raw value (computing its normalised position), without
// disturbing a control the user is editing.
function driveWidget(path, value) {
  const w = widgets.get(path);
  const spec = specsByPath.get(path);
  if (!w || !spec) return;
  const span = (spec.max || 1) - (spec.min || 0) || 1;
  const norm = Math.min(1, Math.max(0, (value - (spec.min || 0)) / span));
  w.set(value, norm);
}

function setMeter(id, v) { document.getElementById(id).style.height = `${Math.min(100, (v || 0) * 100)}%`; }

// --- helpers ---

function fmt(v, spec) {
  if (spec.kind === "int") return String(Math.round(v));
  const s = Math.abs(v) >= 100 ? v.toFixed(0) : v.toFixed(2);
  return spec.unit ? `${s}${spec.unit}` : s;
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

main();
