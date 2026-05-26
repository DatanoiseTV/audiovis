// audiovis web control surface.
//
// Loads the shared control.proto at runtime (single source of truth with the
// Rust server), opens a binary websocket, builds the control UI from the
// parameter schema the server sends, and keeps everything in two-way sync.

"use strict";

// Bump on every UI change so it is obvious in the console whether the browser
// is running fresh assets or a stale cached copy.
const UI_BUILD = "ui-35";
console.log(`audiovis ${UI_BUILD} loaded`);

const BLEND_NAMES = ["normal", "add", "screen", "multiply", "difference"];
const DIV_NAMES = ["8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16"];
const SHAPE_NAMES = ["sine", "triangle", "saw up", "saw dn", "square", "pulse", "rand", "noise", "steps"];
const FONT_NAMES = ["system", "bold", "outline", "alien", "vt323", "silkscreen", "arcade"];
const TEXTFX_NAMES = ["none", "dissolve", "wave", "tear", "scanlines"];

// Named option lists for integer params that read better as dropdowns.
function intOptions(path) {
  if (path.endsWith(".generator")) return generators;
  if (path.endsWith(".source")) return mediaFiles;
  if (path === "wire.mesh") return meshNames;
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
let mediaFiles = ["(none)"];
let meshNames = ["(shapes)"];
let isfShaders = ["(off)"]; // ISF shader file names
let isfInputs = [];          // [{label, kind, slot}] for the selected shader
let modSources = [];
let audioDevices = [], audioDevice = "", midiPorts = [], midiPort = "", cameraDevices = [], cameraDevice = "";
let mediaSourceSelects = []; // {sel, fill} for live source-dropdown refresh
let mediaDeck = 0;           // which media layer the browser loads into
let scriptNames = [];        // available script names (builtins + user)
let layerCards = {};         // layer index -> its card element (dynamic stack)
const MAX_LAYERS = 8;        // must match the compositor's NUM_LAYERS
let fxCards = [];            // {card, enablePath, orderPath, name, pos} per FX instance
let lfoCards = [];           // {card, enablePath, canvas} per LFO
let latestTelemetry = null;
let blackoutPrev = null; // brightness remembered while blacked out
let linkEnabled = false; // Ableton Link state (from telemetry)
let presetList = [];
let currentPreset = "";
const textSlots = [];
const textInputs = new Map(); // slot -> input element
let mappingList = [];
let mapListEl = null;

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
  const lk = document.getElementById("link");
  if (lk) lk.onclick = () => send({ link: { enabled: !linkEnabled } });
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
        if (msg.media && msg.media.length) mediaFiles = msg.media;
        if (msg.meshes && msg.meshes.length) meshNames = msg.meshes;
        if (msg.mod_sources && msg.mod_sources.length) modSources = msg.mod_sources;
        buildUI(msg.schema);
      } else if (msg.media && msg.media.length) {
        // media-only update (a rescan picked up new files)
        mediaFiles = msg.media;
        if (msg.meshes && msg.meshes.length) meshNames = msg.meshes;
        onMediaListChanged();
      }
      if (msg.changes) for (const c of msg.changes) applyChange(c);
      if (msg.telemetry) applyTelemetry(msg.telemetry);
      if (msg.mod_routes_present) renderRoutes(msg.mod_routes || []);
      if (msg.current_preset) currentPreset = msg.current_preset;
      if (msg.presets && msg.presets.length) { presetList = msg.presets; renderPresets(); }
      else if (msg.current_preset) renderPresets();
      if (msg.text && msg.text.length) { for (const t of msg.text) textSlots[t.id] = t.text; refreshTextInputs(); }
      if (msg.mappings_present) { mappingList = msg.mappings || []; renderMappings(); }
      if (msg.devices_present) {
        audioDevices = msg.audio_devices || [];
        audioDevice = msg.audio_device || "";
        midiPorts = msg.midi_ports || [];
        midiPort = msg.midi_port || "";
        cameraDevices = msg.camera_devices || [];
        cameraDevice = msg.camera_device || "";
        renderDevices();
      }
      if (msg.script_present) {
        if (msg.scripts && msg.scripts.length) { scriptNames = msg.scripts; renderScriptNames(); }
        if (msg.script) setScriptEditor(msg.script);
      }
      if (msg.script_error_present) showScriptError(msg.script_error || "");
      if (msg.isf_present && msg.isf_shaders && msg.isf_shaders.length) { isfShaders = msg.isf_shaders; renderIsfShaders(); }
      if (msg.isf_inputs_present) { isfInputs = msg.isf_inputs || []; renderIsfInputs(); showIsfError(msg.isf_error || ""); }
      if (msg.preview && msg.preview.length) updatePreview(msg.preview);
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
const sendRelease = (path) => send({ set: { path, release: true } });
const sendDevice = (kind, name) => send({ device: { kind, name } });
const sendScript = (action, name, source) => send({ script: { action, name: name || "", source: source || "" } });

// --- UI building ---

const FX_GROUPS = ["Feedback", "Mirror", "Hue cycle", "Lo-fi", "VHS", "Glitch", "Bloom"];

// Order groups for a live surface: layers first, then the effects rack, then
// LFOs / clock / global.
function groupOrder(name) {
  if (name.startsWith("Layer")) return 0 + (parseInt(name.replace(/\D/g, ""), 10) || 0);
  if (name.startsWith("Media")) return 5 + (parseInt(name.replace(/\D/g, ""), 10) || 0);
  const fx = FX_GROUPS.indexOf(name);
  if (fx >= 0) return 10 + fx;
  if (name.startsWith("LFO ")) return 35 + (parseInt(name.replace(/\D/g, ""), 10) || 0);
  return { Audio: 28, Clock: 31, Global: 32, Text: 33 }[name] ?? 40;
}

function buildUI(schema) {
  for (const s of schema) specsByPath.set(s.path, s);
  widgets.clear();
  mediaSourceSelects = [];
  const main = document.getElementById("groups");
  main.innerHTML = "";

  const groups = new Map();
  for (const s of schema) {
    if (!groups.has(s.group)) groups.set(s.group, []);
    groups.get(s.group).push(s);
  }

  main.appendChild(buildMonitor()); // live output monitor up top, Resolume-style
  main.appendChild(buildMediaBrowser()); // clip-style media library grid
  layerCards = {};
  fxCards = [];

  // Dedicated racks: layers and the FX chain are managed as decks with inline
  // add/remove, the rest of the controls flow below.
  const layersRack = el("div", "group matrix rack");
  layersRack.appendChild(el("h2", null, "Layers"));
  const layersBody = el("div", "rackbody");
  layersRack.appendChild(layersBody);
  main.appendChild(layersRack);

  const fxRack = el("div", "group matrix rack");
  fxRack.appendChild(el("h2", null, "FX chain"));
  const fxBody = el("div", "rackbody");
  fxRack.appendChild(fxBody);
  main.appendChild(fxRack);

  const lfoRack = el("div", "group matrix rack");
  lfoRack.appendChild(el("h2", null, "LFOs"));
  const lfoBody = el("div", "rackbody");
  lfoRack.appendChild(lfoBody);
  main.appendChild(lfoRack);
  lfoCards = [];

  const names = [...groups.keys()].sort((a, b) => groupOrder(a) - groupOrder(b));
  for (const name of names) {
    if (name === "Text") { main.appendChild(buildLettering(groups.get(name))); continue; }
    if (name === "ISF") continue; // raw pool params; the ISF panel renders these
    const specs = groups.get(name);
    const fxEnable = specs.find((s) => /^post\..*\.enable$/.test(s.path));
    const isLayer = name.startsWith("Layer");
    const cls = isLayer || name.startsWith("Media") ? "group layer" : fxEnable ? "group fx" : "group";
    const sec = el("div", cls);
    const h = el("h2", null, name);
    h.onclick = () => sec.classList.toggle("collapsed"); // click a header to fold it
    sec.appendChild(h);
    // Generator layers are a dynamic stack: tag the card and add a remove (x).
    const lm = name.match(/^Layer (\d+)$/);
    if (lm) {
      const idx = parseInt(lm[1], 10) - 1;
      sec.dataset.layer = idx;
      layerCards[idx] = sec;
      const rm = el("button", "btn rmlayer", "×");
      rm.title = "remove this layer";
      rm.onclick = (e) => { e.stopPropagation(); removeLayer(idx); };
      h.appendChild(rm);
    }
    // FX instances are a dynamic, reorderable chain: header gets up/down + x,
    // and the enable/order rows are managed by those (not shown as sliders).
    let skip = null;
    if (fxEnable) {
      const enablePath = fxEnable.path;
      const orderPath = enablePath.replace(/\.enable$/, ".order");
      skip = new Set([enablePath, orderPath]);
      sec.dataset.fx = enablePath;
      const pos = el("span", "fxpos");
      h.appendChild(pos);
      fxCards.push({ card: sec, enablePath, orderPath, name, pos });
      const up = el("button", "btn fxmove", "▲"); up.title = "move earlier in the chain";
      up.onclick = (e) => { e.stopPropagation(); moveFx(enablePath, -1); };
      const dn = el("button", "btn fxmove", "▼"); dn.title = "move later in the chain";
      dn.onclick = (e) => { e.stopPropagation(); moveFx(enablePath, 1); };
      const rm = el("button", "btn rmlayer", "×"); rm.title = "remove this effect";
      rm.onclick = (e) => { e.stopPropagation(); removeFx(enablePath); };
      const grp = el("span", "fxhdr"); grp.append(up, dn, rm); h.appendChild(grp);
    }
    // LFOs are a dynamic pool too: enable shows the LFO and feeds it as a
    // source; the card carries a live scope and a remove (x).
    const lfoM = name.match(/^LFO (\d+)$/);
    if (lfoM) {
      const n = parseInt(lfoM[1], 10);
      const enablePath = `lfo.${n}.enable`;
      skip = new Set([enablePath]);
      sec.dataset.lfo = enablePath;
      const rm = el("button", "btn rmlayer", "×"); rm.title = "remove this LFO";
      rm.onclick = (e) => { e.stopPropagation(); removeLfo(enablePath); };
      h.appendChild(rm);
      const cv = el("canvas"); cv.width = 250; cv.height = 46; cv.id = `lfoscope-${n}`; cv.className = "lfocanvas";
      lfoCards.push({ card: sec, enablePath, canvas: cv });
      sec._lfoCanvas = cv;
    }

    for (const spec of specs) { if (!skip || !skip.has(spec.path)) sec.appendChild(buildRow(spec)); }
    if (sec._lfoCanvas) sec.appendChild(sec._lfoCanvas);
    // Route layer/FX/LFO cards into their racks; everything else flows below.
    if (lm) layersBody.appendChild(sec);
    else if (fxEnable) fxBody.appendChild(sec);
    else if (lfoM) lfoBody.appendChild(sec);
    else main.appendChild(sec);
  }

  const addLfoTile = el("button", "addtile");
  addLfoTile.id = "addlfo";
  addLfoTile.textContent = "+ Add LFO";
  addLfoTile.onclick = () => addLfo();
  lfoBody.appendChild(addLfoTile);
  refreshLfos();

  // Inline add tiles at the end of each rack.
  const addLayerTile = el("button", "addtile");
  addLayerTile.id = "addlayer";
  addLayerTile.textContent = "+ Add layer";
  addLayerTile.onclick = () => addLayer();
  layersBody.appendChild(addLayerTile);

  const addFxTile = el("div", "addtile");
  const fxSel = el("select"); fxSel.id = "addfx";
  addFxTile.appendChild(fxSel);
  fxBody.appendChild(addFxTile);

  refreshLayers();
  refreshFx();
  main.appendChild(buildDevices());
  main.appendChild(buildScript());
  main.appendChild(buildIsf());
  main.appendChild(buildMappings());
  main.appendChild(buildMatrix());
  main.appendChild(buildPresets());
}

// --- dynamic layer stack (add / remove) ---

// A layer is "active" (and shown) if it has any opacity; layer 0 is always on.
function layerActive(i) {
  if (i === 0) return true;
  const op = paramValues.get(`layer.${i}.opacity`);
  return !!op && op.value > 0.001;
}

// Show only active layer cards; activate the next free one on "+ Layer".
function refreshLayers() {
  let active = 0;
  for (let i = 0; i < MAX_LAYERS; i++) {
    const card = layerCards[i];
    if (!card) continue;
    const on = layerActive(i);
    card.style.display = on ? "" : "none";
    if (on) active++;
  }
  const add = document.getElementById("addlayer");
  if (add) add.disabled = active >= MAX_LAYERS;
}

function addLayer() {
  for (let i = 1; i < MAX_LAYERS; i++) {
    if (!layerActive(i)) {
      sendRaw(`layer.${i}.generator`, i % generators.length); // a non-blank default
      sendRaw(`layer.${i}.blend`, 1); // add, so it shows over the stack
      sendNorm(`layer.${i}.opacity`, 1.0);
      // Reveal immediately; the echoed value keeps it shown.
      if (layerCards[i]) layerCards[i].style.display = "";
      paramValues.set(`layer.${i}.opacity`, { value: 1, norm: 1 });
      refreshLayers();
      return;
    }
  }
}

function removeLayer(i) {
  if (i === 0) return; // keep the base layer
  sendNorm(`layer.${i}.opacity`, 0.0);
  paramValues.set(`layer.${i}.opacity`, { value: 0, norm: 0 });
  if (layerCards[i]) layerCards[i].style.display = "none";
  refreshLayers();
}

// --- dynamic FX chain (add / remove / reorder instances) ---

const fxEnabled = (c) => { const v = paramValues.get(c.enablePath); return !!v && v.value >= 0.5; };
const fxOrder = (c) => Math.round((paramValues.get(c.orderPath) || {}).value || 0);
// Parse the effect type id + instance number from an enable path.
function fxInfo(enablePath) {
  const m = enablePath.match(/^post\.([a-z]+)(?:\.(\d+))?\.enable$/);
  return m ? { id: m[1], inst: m[2] ? parseInt(m[2], 10) : 0 } : { id: enablePath, inst: 0 };
}

// Show only enabled FX cards, in chain order, with their position numbers.
function refreshFx() {
  const active = fxCards.filter(fxEnabled).sort((a, b) => fxOrder(a) - fxOrder(b));
  for (const c of fxCards) c.card.style.display = fxEnabled(c) ? "" : "none";
  active.forEach((c, i) => { if (c.pos) c.pos.textContent = `#${i + 1}`; });
  renderFxAdd(active.length);
}

// Renumber the chain order densely from the current visual order.
function renumberFx(list) {
  list.forEach((c, i) => { sendRaw(c.orderPath, i); paramValues.set(c.orderPath, { value: i, norm: 0 }); });
}

function moveFx(enablePath, dir) {
  const active = fxCards.filter(fxEnabled).sort((a, b) => fxOrder(a) - fxOrder(b));
  const i = active.findIndex((c) => c.enablePath === enablePath);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= active.length) return;
  [active[i], active[j]] = [active[j], active[i]];
  renumberFx(active);
  refreshFx();
}

function removeFx(enablePath) {
  sendRaw(enablePath, 0);
  paramValues.set(enablePath, { value: 0, norm: 0 });
  refreshFx();
}

// Enable the lowest free instance of an effect type and append it to the chain.
function addFx(id) {
  const insts = fxCards.filter((c) => fxInfo(c.enablePath).id === id).sort((a, b) => fxInfo(a.enablePath).inst - fxInfo(b.enablePath).inst);
  const free = insts.find((c) => !fxEnabled(c));
  if (!free) return;
  const tail = fxCards.filter(fxEnabled).reduce((m, c) => Math.max(m, fxOrder(c)), -1) + 1;
  sendRaw(free.orderPath, tail); paramValues.set(free.orderPath, { value: tail, norm: 0 });
  sendRaw(free.enablePath, 1); paramValues.set(free.enablePath, { value: 1, norm: 1 });
  refreshFx();
}

// The "+ FX" picker lists the effect types (deduped from the FX instances).
function renderFxAdd() {
  const sel = document.getElementById("addfx");
  if (!sel) return;
  if (!sel.dataset.built) {
    const seen = new Set();
    sel.innerHTML = "";
    const def = el("option", null, "+ FX"); def.value = ""; sel.appendChild(def);
    for (const c of fxCards) {
      const id = fxInfo(c.enablePath).id;
      if (seen.has(id)) continue; seen.add(id);
      const label = c.name.replace(/ \d+$/, "");
      const o = el("option", null, label); o.value = id; sel.appendChild(o);
    }
    sel.dataset.built = "1";
    sel.onchange = () => { if (sel.value) { addFx(sel.value); sel.value = ""; } };
  }
}

// --- dynamic LFO pool (add / remove) ---

const lfoEnabled = (c) => { const v = paramValues.get(c.enablePath); return !!v && v.value >= 0.5; };

function refreshLfos() {
  let active = 0;
  for (const c of lfoCards) { const on = lfoEnabled(c); c.card.style.display = on ? "" : "none"; if (on) active++; }
  const add = document.getElementById("addlfo");
  if (add) add.disabled = active >= lfoCards.length;
}

function addLfo() {
  const free = lfoCards.find((c) => !lfoEnabled(c));
  if (!free) return;
  sendRaw(free.enablePath, 1);
  paramValues.set(free.enablePath, { value: 1, norm: 1 });
  refreshLfos();
}

function removeLfo(enablePath) {
  sendRaw(enablePath, 0);
  paramValues.set(enablePath, { value: 0, norm: 0 });
  refreshLfos();
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
  for (const c of lfoCards) {
    if (c.card.style.display === "none") continue;
    const cv = c.canvas;
    const n = parseInt(c.enablePath.split(".")[1], 10);
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
    const i = el("input"); i.type = "range"; i.min = min; i.max = 1; i.step = "any"; i.value = val;
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
    const fill = () => { sel.innerHTML = ""; intOptions(spec.path).forEach((nm, i) => { const o = el("option", null, nm); o.value = i; sel.appendChild(o); }); };
    fill();
    sel.onchange = () => sendRaw(spec.path, parseInt(sel.value, 10));
    mid.appendChild(sel);
    val.style.display = "none";
    // Media/mesh dropdowns are refreshed live when their file list changes.
    if (spec.path.endsWith(".source") || spec.path === "wire.mesh") mediaSourceSelects.push({ sel, fill });
    widget = { set: (v) => { sel.value = Math.round(v); } };
  } else {
    // float or generic int: a normalised slider, value display shows raw.
    // step "any" gives full float resolution while dragging.
    const r = el("input"); r.type = "range"; r.min = 0; r.max = 1; r.step = "any";
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
    const path = `text.${n}.trigger`;
    const row = el("div", "textrow");
    const inp = el("input"); inp.type = "text"; inp.placeholder = `slot ${n + 1}`; inp.value = textSlots[n] || "";
    inp.oninput = () => sendText(n, inp.value);
    // Momentary: shown while held (mirrors a MIDI note's on/off gate).
    const show = el("button", "btn", "hold");
    show.title = "hold to show; bind a MIDI note via L for note-on/off gating";
    show.onpointerdown = (e) => { e.preventDefault(); sendTrigger(path); };
    show.onpointerup = () => sendRelease(path);
    show.onpointerleave = () => sendRelease(path);
    // Learn: arm this slot, then a MIDI note gates it (Trigger params learn as Gate).
    const learn = el("button", "btn learn", "L");
    learn.onclick = () => {
      const arming = armed !== path;
      document.querySelectorAll(".learn.armed").forEach((b) => b.classList.remove("armed"));
      if (arming) { sendLearn(path, true, false); setArmed(path); learn.classList.add("armed"); }
      else { sendLearn(path, false, false); setArmed(null); }
    };
    row.append(inp, show, learn);
    sec.appendChild(row);
    textInputs.set(n, inp);
  }
  const clr = el("button", "btn", "clear all"); clr.onclick = () => sendTrigger("text.clear");
  sec.appendChild(clr);
  // Style params (font / fx / size / pos / hue); the per-slot triggers are the
  // "show" buttons above, so skip trigger kinds here.
  for (const spec of specs) {
    if (spec.kind === "trigger") continue;
    sec.appendChild(buildRow(spec));
  }
  return sec;
}

// --- media browser (clip-style library that loads into the media decks) ---

let mediaGridEl = null;

// A grid of thumbnails for everything in the media folder; click a tile to load
// it into the selected media deck (Media 1 / Media 2), like a clip bank.
function buildMediaBrowser() {
  const sec = el("div", "group matrix mediabrowser");
  const h = el("h2", null, "Media browser");
  h.onclick = (e) => { if (e.target === h) sec.classList.toggle("collapsed"); };
  sec.appendChild(h);

  const bar = el("div", "mbbar");
  bar.appendChild(el("span", "gl", "Load into"));
  const mk = (label, deck) => {
    const b = el("button", "btn deckbtn" + (mediaDeck === deck ? " on" : ""), label);
    b.dataset.deck = deck;
    b.onclick = () => { mediaDeck = deck; renderDeckBtns(bar); renderMediaGrid(); };
    return b;
  };
  bar.appendChild(mk("Deck 1", 0));
  bar.appendChild(mk("Deck 2", 1));
  const spacer = el("span"); spacer.style.flex = "1"; bar.appendChild(spacer);
  const rescan = el("button", "btn", "Rescan folder");
  rescan.title = "pick up files you just dropped into the media folder";
  rescan.onclick = () => send({ rescan_media: true });
  bar.appendChild(rescan);
  sec.appendChild(bar);

  mediaGridEl = el("div", "mbgrid");
  sec.appendChild(mediaGridEl);
  renderMediaGrid();
  return sec;
}

function renderDeckBtns(bar) {
  bar.querySelectorAll(".deckbtn").forEach((b) => b.classList.toggle("on", +b.dataset.deck === mediaDeck));
}

// Set the active deck's source to a file index, and reveal it if hidden.
function loadClip(idx) {
  sendRaw(`media.${mediaDeck}.source`, idx);
  const op = paramValues.get(`media.${mediaDeck}.opacity`);
  if (idx > 0 && (!op || op.value < 0.01)) sendNorm(`media.${mediaDeck}.opacity`, 1.0);
  setTimeout(renderMediaGrid, 60);
}

function renderMediaGrid() {
  if (!mediaGridEl) return;
  mediaGridEl.innerHTML = "";
  const cur = Math.round((paramValues.get(`media.${mediaDeck}.source`) || {}).value || 0);
  const clear = el("div", "clip clear" + (cur === 0 ? " on" : ""));
  clear.appendChild(el("div", "cliplbl", "none"));
  clear.onclick = () => loadClip(0);
  mediaGridEl.appendChild(clear);
  for (let i = 1; i < mediaFiles.length; i++) {
    const tile = el("div", "clip" + (cur === i ? " on" : ""));
    const img = el("img"); img.loading = "lazy"; img.src = "media/" + encodeURIComponent(mediaFiles[i]);
    tile.appendChild(img);
    tile.appendChild(el("div", "cliplbl", mediaFiles[i]));
    tile.onclick = () => loadClip(i);
    mediaGridEl.appendChild(tile);
  }
}

// Called when the server sends a fresh media list (after a rescan).
function onMediaListChanged() {
  renderMediaGrid();
  for (const { fill, sel } of mediaSourceSelects) { const v = sel.value; fill(); sel.value = v; }
}

// --- live output monitor ---

let _previewUrl = null;

// Build the output monitor card: a 16:9 frame showing the live render, fed by
// JPEG frames the engine streams over the websocket at ~10 fps.
function buildMonitor() {
  const sec = el("div", "group monitor");
  const h = el("h2", null, "Output");
  h.appendChild(el("span", "live", "")); // pulses when frames arrive
  sec.appendChild(h);
  const frame = el("div", "screen");
  const img = el("img");
  img.id = "preview";
  img.alt = "live output";
  frame.appendChild(img);
  sec.appendChild(frame);
  const grip = el("div", "reszh"); // bottom-right resize handle
  sec.appendChild(grip);
  makeMonitorInteractive(sec, h, grip);
  return sec;
}

// Drag the monitor by its header and resize it from the corner grip; a header
// click that doesn't move folds it. Position/size persist across reloads.
function makeMonitorInteractive(sec, header, grip) {
  const saved = (() => { try { return JSON.parse(localStorage.getItem("av.monitor") || "{}"); } catch { return {}; } })();
  if (saved.w) sec.style.width = saved.w + "px";
  if (saved.left != null) { sec.style.left = saved.left + "px"; sec.style.top = saved.top + "px"; sec.style.right = "auto"; }
  if (saved.collapsed) sec.classList.add("collapsed");
  const store = () => {
    const r = sec.getBoundingClientRect();
    const usingLeft = sec.style.right === "auto";
    localStorage.setItem("av.monitor", JSON.stringify({
      w: Math.round(r.width),
      left: usingLeft ? Math.round(r.left) : null,
      top: usingLeft ? Math.round(r.top) : null,
      collapsed: sec.classList.contains("collapsed"),
    }));
  };

  // --- drag from the header ---
  header.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    const r = sec.getBoundingClientRect();
    const ox = e.clientX - r.left, oy = e.clientY - r.top;
    let moved = false;
    header.setPointerCapture(e.pointerId);
    const move = (ev) => {
      if (Math.abs(ev.clientX - e.clientX) + Math.abs(ev.clientY - e.clientY) > 3) moved = true;
      const x = Math.max(0, Math.min(window.innerWidth - 40, ev.clientX - ox));
      const y = Math.max(0, Math.min(window.innerHeight - 30, ev.clientY - oy));
      sec.style.left = x + "px"; sec.style.top = y + "px"; sec.style.right = "auto";
    };
    const up = () => {
      header.removeEventListener("pointermove", move);
      header.removeEventListener("pointerup", up);
      if (!moved) sec.classList.toggle("collapsed"); // a plain click folds it
      store();
    };
    header.addEventListener("pointermove", move);
    header.addEventListener("pointerup", up);
  });

  // --- resize from the corner grip ---
  grip.addEventListener("pointerdown", (e) => {
    e.stopPropagation(); e.preventDefault();
    const startW = sec.getBoundingClientRect().width, sx = e.clientX;
    grip.setPointerCapture(e.pointerId);
    const move = (ev) => {
      sec.style.width = Math.max(180, Math.min(900, startW + (ev.clientX - sx))) + "px";
    };
    const up = () => { grip.removeEventListener("pointermove", move); grip.removeEventListener("pointerup", up); store(); };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
  });
}

// Swap the monitor image to the newest frame, releasing the previous blob URL
// once the new one has loaded so we don't leak object URLs over a long set.
function updatePreview(bytes) {
  const img = document.getElementById("preview");
  if (!img) return;
  const url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
  img.onload = () => {
    if (_previewUrl) URL.revokeObjectURL(_previewUrl);
    _previewUrl = url;
  };
  img.src = url;
  const mon = img.closest(".monitor");
  if (mon) { mon.classList.add("lit"); clearTimeout(mon._lt); mon._lt = setTimeout(() => mon.classList.remove("lit"), 250); }
}

// --- audio / MIDI input device selection ---

let devicesEl = null;

// Build the I/O panel: dropdowns to pick the live audio input and MIDI input.
// The audio analyzer knobs (gain/attack/release/sensitivity) render in their
// own "Audio" param group next to it.
function buildDevices() {
  const sec = el("div", "group io");
  sec.appendChild(el("h2", null, "Input / Output"));
  devicesEl = el("div");
  sec.appendChild(devicesEl);
  renderDevices();
  return sec;
}

function devicePicker(label, kind, options, current, defaultLabel) {
  const row = el("div", "row");
  row.appendChild(el("div", "name", label));
  const sel = el("select");
  const def = el("option", null, defaultLabel); def.value = ""; sel.appendChild(def);
  for (const name of options) { const o = el("option", null, name); o.value = name; sel.appendChild(o); }
  // Match the current selection: server sends a substring filter for MIDI, the
  // device name for audio - select the option that contains/equals it.
  sel.value = options.includes(current) ? current : "";
  sel.onchange = () => sendDevice(kind, sel.value);
  const mid = el("div"); mid.appendChild(sel);
  row.appendChild(mid);
  row.appendChild(el("div"));
  return row;
}

function renderDevices() {
  if (!devicesEl) return;
  devicesEl.innerHTML = "";
  devicesEl.appendChild(devicePicker("Audio in", "audio", audioDevices, audioDevice, "system default"));
  devicesEl.appendChild(devicePicker("MIDI in", "midi", midiPorts, midiPort, "all ports"));
  devicesEl.appendChild(devicePicker("Camera", "camera", cameraDevices, cameraDevice,
    cameraDevices.length ? "first available" : "(camera feature off)"));
}

// --- JS scripting panel ---

let scriptEditor = null, scriptNameInput = null, scriptLoadSel = null, scriptErrEl = null;

// A code editor that runs JS every frame to automate params or draw into the
// 2D buffer (shown by the "script" generator). Apply runs live; Save stores it.
function buildScript() {
  const sec = el("div", "group matrix scriptpanel");
  const h = el("h2", null, "Script");
  h.onclick = (e) => { if (e.target === h) sec.classList.toggle("collapsed"); };
  sec.appendChild(h);

  sec.appendChild(el("div", "hint",
    "Runs every frame. In scope: t, dt, frame, low, mid, high, rms, onset, beat, bar, bpm, lfo[0..5]. " +
    "Calls: set(path,v) setn(path,0..1) get(path) trigger(path) | 2D: clear(r,g,b) pset(x,y,r,g,b) line(...) rect(...), SW x SH — select the \"script\" generator. " +
    "The 2D buffer suits shapes/sprites; heavy per-pixel loops are slow (use the GLSL generators for those)."));

  scriptEditor = el("textarea", "scripted");
  scriptEditor.spellcheck = false;
  scriptEditor.placeholder = "// e.g.\nset(\"layer.0.hue\", 0.5 + 0.5*Math.sin(t));\nset(\"global.brightness\", 0.7 + rms*0.3);";
  // Ctrl/Cmd-Enter applies.
  scriptEditor.onkeydown = (e) => { if ((e.ctrlKey || e.metaKey) && e.key === "Enter") { e.preventDefault(); applyScript(); } };
  sec.appendChild(scriptEditor);

  scriptErrEl = el("div", "scripterr");
  sec.appendChild(scriptErrEl);

  const bar = el("div", "scriptbar");
  const apply = el("button", "btn", "Apply (Cmd/Ctrl-Enter)");
  apply.onclick = applyScript;
  bar.appendChild(apply);
  const stop = el("button", "btn", "Stop");
  stop.title = "clear the running script";
  stop.onclick = () => { sendScript("apply", "", ""); };
  bar.appendChild(stop);

  const show = el("button", "btn", "Show 2D output");
  show.title = "point Layer 1 at the 'script' generator so a drawing script is visible";
  show.onclick = () => {
    const gi = generators.indexOf("script");
    if (gi < 0) return;
    sendRaw("layer.0.generator", gi);
    sendNorm("layer.0.opacity", 1.0);
    paramValues.set("layer.0.opacity", { value: 1, norm: 1 });
    refreshLayers();
  };
  bar.appendChild(show);

  const spacer = el("span"); spacer.style.flex = "1"; bar.appendChild(spacer);

  scriptLoadSel = el("select");
  scriptLoadSel.onchange = () => { if (scriptLoadSel.value) { sendScript("load", scriptLoadSel.value, ""); scriptNameInput.value = scriptLoadSel.value; } };
  renderScriptNames();
  bar.appendChild(scriptLoadSel);

  scriptNameInput = el("input"); scriptNameInput.type = "text"; scriptNameInput.placeholder = "name"; scriptNameInput.className = "scriptname";
  bar.appendChild(scriptNameInput);
  const save = el("button", "btn", "Save");
  save.onclick = () => { const n = (scriptNameInput.value || "").trim(); if (n) sendScript("save", n, scriptEditor.value); };
  bar.appendChild(save);

  sec.appendChild(bar);
  return sec;
}

function applyScript() { if (scriptEditor) sendScript("apply", "", scriptEditor.value); }
function setScriptEditor(src) { if (scriptEditor) scriptEditor.value = src; }
function renderScriptNames() {
  if (!scriptLoadSel) return;
  scriptLoadSel.innerHTML = "";
  const def = el("option", null, "load example..."); def.value = ""; scriptLoadSel.appendChild(def);
  for (const n of scriptNames) { const o = el("option", null, n); o.value = n; scriptLoadSel.appendChild(o); }
}
function showScriptError(err) {
  if (!scriptErrEl) return;
  scriptErrEl.textContent = err;
  scriptErrEl.classList.toggle("on", !!err);
}

// --- ISF (interactive shader format) panel ---

let isfShaderSel = null, isfInputsEl = null, isfErrEl = null;

// Pick an ISF shader (put it on a layer's "isf" generator) and tune its inputs.
function buildIsf() {
  const sec = el("div", "group matrix isfpanel");
  const h = el("h2", null, "ISF shaders");
  h.onclick = (e) => { if (e.target === h) sec.classList.toggle("collapsed"); };
  sec.appendChild(h);
  sec.appendChild(el("div", "hint",
    "Drop ISF .fs files into the isf/ folder (Rescan in the media browser), set a layer's Generator to \"isf\", then pick a shader. Inputs below are live + modulatable. Some isf.video shaders need GL3/multi-pass and may not compile on GLES2."));

  const row = el("div", "row");
  row.appendChild(el("div", "name", "Shader"));
  const sel = el("select");
  sel.onchange = () => { sendRaw("isf.shader", parseInt(sel.value, 10)); paramValues.set("isf.shader", { value: parseInt(sel.value, 10), norm: 0 }); };
  isfShaderSel = sel;
  renderIsfShaders();
  const mid = el("div"); mid.appendChild(sel); row.appendChild(mid); row.appendChild(el("div"));
  sec.appendChild(row);

  isfErrEl = el("div", "scripterr");
  sec.appendChild(isfErrEl);
  isfInputsEl = el("div");
  sec.appendChild(isfInputsEl);
  renderIsfInputs();
  return sec;
}

function renderIsfShaders() {
  if (!isfShaderSel) return;
  isfShaderSel.innerHTML = "";
  isfShaders.forEach((nm, i) => { const o = el("option", null, nm); o.value = i; isfShaderSel.appendChild(o); });
  const cur = paramValues.get("isf.shader");
  if (cur) isfShaderSel.value = Math.round(cur.value);
}

// Render the selected shader's inputs as labeled sliders bound to the isf.<slot>
// pool params (so they update live and can be MIDI-mapped / modulated).
function renderIsfInputs() {
  if (!isfInputsEl) return;
  isfInputsEl.innerHTML = "";
  for (const inp of isfInputs) {
    const count = inp.kind === "color" ? 4 : inp.kind === "point2D" ? 2 : 1;
    const suffix = inp.kind === "color" ? [" R", " G", " B", " A"] : inp.kind === "point2D" ? [" X", " Y"] : [""];
    for (let k = 0; k < count; k++) {
      const spec = specsByPath.get(`isf.${inp.slot + k}`);
      if (!spec) continue;
      isfInputsEl.appendChild(buildRow({ ...spec, name: inp.label + suffix[k] }));
    }
  }
}

function showIsfError(err) {
  if (!isfErrEl) return;
  isfErrEl.textContent = err;
  isfErrEl.classList.toggle("on", !!err);
}

// --- MIDI / OSC mapping list ---

function buildMappings() {
  const sec = el("div", "group matrix");
  sec.appendChild(el("h2", null, "MIDI / OSC map"));
  sec.appendChild(el("div", "hint", "arm any control's L, then move a knob / press a note to bind it"));
  mapListEl = el("div", "maplist");
  sec.appendChild(mapListEl);
  renderMappings();
  return sec;
}

function renderMappings() {
  if (!mapListEl) return;
  mapListEl.innerHTML = "";
  if (!mappingList.length) {
    mapListEl.appendChild(el("div", "empty", "no bindings yet"));
    return;
  }
  for (const m of mappingList) {
    const row = el("div", "maprow");
    const tgt = specsByPath.get(m.target);
    row.appendChild(el("div", "src", m.source));
    row.appendChild(el("div", "rlabel", `${tgt ? tgt.group + " / " + tgt.name : m.target}  ·  ${m.mode}`));
    const rm = el("button", "btn", "x");
    rm.title = "remove binding";
    rm.onclick = () => sendLearn(m.target, false, true); // clear mappings for this target
    row.appendChild(rm);
    mapListEl.appendChild(row);
  }
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
  // Reflect a deck's loaded clip in the media browser highlight.
  if (c.path === `media.${mediaDeck}.source`) renderMediaGrid();
  // A layer's opacity arriving (e.g. from a preset) toggles its card.
  if (/^layer\.\d+\.opacity$/.test(c.path)) refreshLayers();
  // An FX enable/order arriving (e.g. from a preset) updates the chain view.
  if (/^post\..*\.(enable|order)$/.test(c.path)) refreshFx();
  // An LFO enable arriving updates the LFO rack.
  if (/^lfo\.\d+\.enable$/.test(c.path)) refreshLfos();
  // Keep the ISF shader dropdown in sync (e.g. from a preset).
  if (c.path === "isf.shader" && isfShaderSel) isfShaderSel.value = Math.round(c.value || 0);
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

  // Ableton Link button: lit when enabled, with the peer count.
  linkEnabled = !!t.link_enabled;
  const lb = document.getElementById("link");
  if (lb) {
    lb.classList.toggle("active", linkEnabled);
    lb.textContent = linkEnabled ? `LINK · ${t.link_peers || 0}` : "LINK";
  }
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
  const a = Math.abs(v);
  const s = a >= 100 ? v.toFixed(1) : a >= 10 ? v.toFixed(2) : v.toFixed(3);
  return spec.unit ? `${s}${spec.unit}` : s;
}

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

main();
