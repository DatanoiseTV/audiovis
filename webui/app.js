// audiovis web control surface.
//
// Loads the shared control.proto at runtime (single source of truth with the
// Rust server), opens a binary websocket, builds the control UI from the
// parameter schema the server sends, and keeps everything in two-way sync.

"use strict";

// Bump on every UI change so it is obvious in the console whether the browser
// is running fresh assets or a stale cached copy.
const UI_BUILD = "ui-9";
console.log(`audiovis ${UI_BUILD} loaded`);

const BLEND_NAMES = ["normal", "add", "screen", "multiply", "difference"];
const DIV_NAMES = ["8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16"];
const SHAPE_NAMES = ["sine", "triangle", "saw", "square", "s&h"];

// Named option lists for integer params that read better as dropdowns.
function intOptions(path) {
  if (path.endsWith(".generator")) return generators;
  if (path.endsWith(".blend")) return BLEND_NAMES;
  if (path.endsWith(".div")) return DIV_NAMES;
  if (path.endsWith(".shape")) return SHAPE_NAMES;
  return null;
}

let ServerMsg, ClientMsg;
let ws = null;
let armed = null; // path currently in MIDI/OSC learn
const widgets = new Map(); // path -> { set(value, norm), spec }
const specsByPath = new Map();
let generators = [];
let modSources = [];
let routesEl = null; // container for the active modulation routes

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
  connect();
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

// --- UI building ---

function buildUI(schema) {
  for (const s of schema) specsByPath.set(s.path, s);
  widgets.clear();
  const main = document.getElementById("groups");
  main.innerHTML = "";

  // Group specs, preserving server order.
  const groups = new Map();
  for (const s of schema) {
    if (!groups.has(s.group)) groups.set(s.group, []);
    groups.get(s.group).push(s);
  }

  for (const [name, specs] of groups) {
    const sec = el("div", "group");
    sec.appendChild(el("h2", null, name));
    for (const spec of specs) sec.appendChild(buildRow(spec));
    main.appendChild(sec);
  }
  main.appendChild(buildMatrix());
  main.appendChild(buildPresetBar());
}

// --- modulation matrix / mixer view ---

const sendMod = (source, target, amount, smooth) => send({ mod: { source, target, amount, smooth } });

// Live registry of route rows so updates are applied in place rather than
// rebuilding the list - otherwise a server rebroadcast would destroy the slider
// being dragged. Keyed by "source|target".
const routeRows = new Map();

function floatTargets() {
  const out = [];
  for (const s of specsByPath.values()) if (s.kind === "float") out.push(s);
  return out;
}

function buildMatrix() {
  const sec = el("div", "group matrix");
  sec.appendChild(el("h2", null, "Modulation matrix"));

  // Add-route row: source -> target @ amount, with smoothing.
  const add = el("div", "modadd");
  const src = el("select");
  modSources.forEach((s) => { const o = el("option", null, s); o.value = s; src.appendChild(o); });
  const tgt = el("select");
  floatTargets().forEach((s) => { const o = el("option", null, `${s.group} / ${s.name}`); o.value = s.path; tgt.appendChild(o); });
  const amt = labelledRange("amt", -1, 1, 0.5);
  const sm = labelledRange("smooth", 0, 1, 0.2);
  const addBtn = el("button", "btn", "+ route");
  addBtn.onclick = () => { if (src.value && tgt.value) sendMod(src.value, tgt.value, parseFloat(amt.input.value) || 0.5, parseFloat(sm.input.value)); };
  add.append(src, el("span", "arrow", "->"), tgt, amt.wrap, sm.wrap, addBtn);
  sec.appendChild(add);

  routeRows.clear();
  routesEl = el("div", "routes");
  sec.appendChild(routesEl);
  return sec;
}

function labelledRange(name, min, max, val) {
  const wrap = el("div", "rng");
  const input = el("input"); input.type = "range"; input.min = min; input.max = max; input.step = 0.01; input.value = val;
  wrap.append(el("span", "rngl", name), input);
  return { wrap, input };
}

function renderRoutes(routes) {
  if (!routesEl) return;
  const seen = new Set();
  routesEl.querySelector(".empty")?.remove();

  for (const r of routes) {
    const key = `${r.source}|${r.target}`;
    seen.add(key);
    let row = routeRows.get(key);
    if (!row) {
      row = buildRouteRow(r);
      routeRows.set(key, row);
      routesEl.appendChild(row.el);
    }
    // Update sliders only when the user is not interacting with them.
    if (document.activeElement !== row.amt) row.amt.value = r.amount || 0;
    if (document.activeElement !== row.smooth) row.smooth.value = r.smooth || 0;
  }
  // Drop rows whose route no longer exists.
  for (const [key, row] of routeRows) {
    if (!seen.has(key)) { row.el.remove(); routeRows.delete(key); }
  }
  if (!routeRows.size && !routesEl.querySelector(".empty")) {
    routesEl.appendChild(el("div", "empty", "no routes - add one above"));
  }
}

function buildRouteRow(r) {
  const el_ = el("div", "route");
  const tgtSpec = specsByPath.get(r.target);
  el_.appendChild(el("div", "rlabel", `${r.source} -> ${tgtSpec ? tgtSpec.name : r.target}`));
  const send = () => sendMod(r.source, r.target, parseFloat(amt.value), parseFloat(smooth.value));

  const amt = el("input"); amt.type = "range"; amt.min = -1; amt.max = 1; amt.step = 0.01; amt.value = r.amount || 0;
  amt.title = "amount"; amt.oninput = send;
  const smooth = el("input"); smooth.type = "range"; smooth.min = 0; smooth.max = 1; smooth.step = 0.01; smooth.value = r.smooth || 0;
  smooth.title = "smoothing"; smooth.oninput = send;

  el_.append(amt, smooth);
  const rm = el("button", "btn", "x");
  rm.title = "remove route";
  rm.onclick = () => sendMod(r.source, r.target, 0, 0);
  el_.appendChild(rm);
  return { el: el_, amt, smooth };
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

function buildPresetBar() {
  const sec = el("div", "group");
  sec.appendChild(el("h2", null, "Preset"));
  const bar = el("div", "preset-bar");
  const input = el("input"); input.type = "text"; input.placeholder = "presets/my.json"; input.value = "presets/live.json";
  const save = el("button", "btn", "save"); save.onclick = () => sendPreset("save", input.value);
  const load = el("button", "btn", "load"); load.onclick = () => sendPreset("load", input.value);
  bar.append(input, save, load);
  sec.appendChild(bar);
  return sec;
}

function setArmed(path) {
  armed = path;
  for (const w of widgets.values()) if (w.learnBtn) w.learnBtn.classList.toggle("armed", w === widgets.get(path));
}

// --- incoming state ---

function applyChange(c) {
  const w = widgets.get(c.path);
  if (w) w.set(c.value || 0, c.norm || 0);
  // A learned binding clears the armed state.
  if (armed === c.path) {} // value updates don't disarm; learn notice would.
}

function applyTelemetry(t) {
  setMeter("m-low", t.low); setMeter("m-mid", t.mid); setMeter("m-high", t.high); setMeter("m-rms", t.rms);
  const beat = document.getElementById("beat");
  // Flash on the audio onset or on each clock downbeat, so it visibly ticks.
  const onDownbeat = (t.beat_phase || 0) < 0.12;
  beat.classList.toggle("hit", (t.beat || 0) > 0.5 || onDownbeat);
  document.getElementById("bpm").textContent = `${Math.round(t.bpm || 0)} BPM`;

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
