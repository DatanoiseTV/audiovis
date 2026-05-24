// audiovis web control surface.
//
// Loads the shared control.proto at runtime (single source of truth with the
// Rust server), opens a binary websocket, builds the control UI from the
// parameter schema the server sends, and keeps everything in two-way sync.

"use strict";

// Bump on every UI change so it is obvious in the console whether the browser
// is running fresh assets or a stale cached copy.
const UI_BUILD = "ui-10";
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
