// audiovis web control surface.
//
// Loads the shared control.proto at runtime (single source of truth with the
// Rust server), opens a binary websocket, builds the control UI from the
// parameter schema the server sends, and keeps everything in two-way sync.

"use strict";

const BLEND_NAMES = ["normal", "add", "screen", "multiply", "difference"];

let ServerMsg, ClientMsg;
let ws = null;
let armed = null; // path currently in MIDI/OSC learn
const widgets = new Map(); // path -> { set(value, norm), spec }
const specsByPath = new Map();
let generators = [];
let modSources = [];
let routesEl = null; // container for the active modulation routes

async function main() {
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

const sendMod = (source, target, amount) => send({ mod: { source, target, amount } });

function floatTargets() {
  const out = [];
  for (const s of specsByPath.values()) if (s.kind === "float") out.push(s);
  return out;
}

function buildMatrix() {
  const sec = el("div", "group matrix");
  sec.appendChild(el("h2", null, "Modulation matrix"));

  // Add-route row: source -> target @ amount.
  const add = el("div", "modadd");
  const src = el("select");
  modSources.forEach((s) => { const o = el("option", null, s); o.value = s; src.appendChild(o); });
  const tgt = el("select");
  floatTargets().forEach((s) => { const o = el("option", null, `${s.group} / ${s.name}`); o.value = s.path; tgt.appendChild(o); });
  const amt = el("input"); amt.type = "range"; amt.min = -1; amt.max = 1; amt.step = 0.01; amt.value = 0.5;
  const addBtn = el("button", "btn", "+ route");
  addBtn.onclick = () => { if (src.value && tgt.value) sendMod(src.value, tgt.value, parseFloat(amt.value) || 0.5); };
  add.append(src, el("span", "arrow", "->"), tgt, amt, addBtn);
  sec.appendChild(add);

  routesEl = el("div", "routes");
  sec.appendChild(routesEl);
  return sec;
}

function renderRoutes(routes) {
  if (!routesEl) return;
  routesEl.innerHTML = "";
  if (!routes.length) {
    routesEl.appendChild(el("div", "empty", "no routes - add one above"));
    return;
  }
  for (const r of routes) {
    const row = el("div", "route");
    const tgtSpec = specsByPath.get(r.target);
    const label = `${r.source} -> ${tgtSpec ? tgtSpec.name : r.target}`;
    row.appendChild(el("div", "rlabel", label));
    const amt = el("input"); amt.type = "range"; amt.min = -1; amt.max = 1; amt.step = 0.01; amt.value = r.amount || 0;
    amt.oninput = () => sendMod(r.source, r.target, parseFloat(amt.value));
    row.appendChild(amt);
    const rm = el("button", "btn", "x");
    rm.title = "remove route";
    rm.onclick = () => sendMod(r.source, r.target, 0);
    row.appendChild(rm);
    routesEl.appendChild(row);
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
  } else if (spec.kind === "int" && (spec.path.endsWith(".generator") || spec.path.endsWith(".blend"))) {
    const sel = el("select");
    const opts = spec.path.endsWith(".generator") ? generators : BLEND_NAMES;
    opts.forEach((nm, i) => { const o = el("option", null, `${i} ${nm}`); o.value = i; sel.appendChild(o); });
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
  beat.classList.toggle("hit", (t.beat || 0) > 0.5);
  document.getElementById("bpm").textContent = `${Math.round(t.bpm || 0)} BPM`;
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
