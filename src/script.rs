//! Embedded JS scripting runtime.
//!
//! A user script runs once per frame with the live signals in scope (audio
//! bands, RMS, onset, beat clock, LFOs, time) and can drive any parameter
//! (`set` / `setn` / `trigger`), read the current value (`get`) and draw into a
//! small RGBA pixel buffer (`clear` / `pset` / `line` / `rect`) that the
//! "Script" generator displays. It is powered by the pure-Rust `boa` engine and
//! is compiled out entirely when the `script` feature is disabled.
//!
//! Host state is shared with the native functions through a thread-local (the
//! runtime only ever runs on the render thread), which sidesteps the GC's
//! capture requirements and keeps the bindings plain function pointers.

/// Pixel-buffer dimensions for the script's 2D drawing surface (16:9).
pub const SCRIPT_W: usize = 160;
pub const SCRIPT_H: usize = 90;

/// Signals handed to the script each frame.
#[derive(Default, Clone, Copy)]
pub struct ScriptSignals {
    pub t: f32,
    pub dt: f32,
    pub frame: f64,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub rms: f32,
    pub onset: f32,
    pub beat: f32,
    pub bar: f32,
    pub bpm: f32,
    pub lfos: [f32; 6],
}

/// A parameter write requested by the script, applied to the engine after the
/// frame's script has run.
pub enum ScriptAction {
    Set(String, f32),
    SetNorm(String, f32),
    Trigger(String),
}

/// What one frame of the script produced.
#[derive(Default)]
pub struct ScriptOutcome {
    pub actions: Vec<ScriptAction>,
    /// True if the script drew into the pixel buffer this frame.
    pub buffer_used: bool,
    /// A new error string to surface to the UI, if one occurred this frame.
    pub error: Option<String>,
}

#[cfg(feature = "script")]
pub use imp::ScriptEngine;

#[cfg(not(feature = "script"))]
pub use stub::ScriptEngine;

/// Example scripts shipped inside the binary.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/scripts/"]
struct BuiltinScripts;

/// Stores JS scripts: curated examples embedded in the binary plus user scripts
/// saved under `scripts/` next to the working directory (a user file shadows a
/// builtin of the same name). Mirrors the preset store.
pub struct ScriptStore {
    dir: std::path::PathBuf,
}

impl Default for ScriptStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptStore {
    pub fn new() -> Self {
        Self { dir: std::path::PathBuf::from("scripts") }
    }

    fn user_path(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(format!("{name}.js"))
    }

    /// All script names: builtins plus user files, de-duplicated and sorted.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> =
            BuiltinScripts::iter().filter_map(|f| f.strip_suffix(".js").map(str::to_string)).collect();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("js") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        names.push(stem.to_string());
                    }
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// Resolve a script's source by name: a user file wins over a builtin.
    pub fn load(&self, name: &str) -> Option<String> {
        let user = self.user_path(name);
        if user.exists() {
            if let Ok(s) = std::fs::read_to_string(&user) {
                return Some(s);
            }
        }
        BuiltinScripts::get(&format!("{name}.js")).map(|f| String::from_utf8_lossy(&f.data).into_owned())
    }

    /// Save a script to the user directory.
    pub fn save(&self, name: &str, source: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.user_path(name), source)
    }
}

/// Real implementation, present when the `script` feature is on.
#[cfg(feature = "script")]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use boa_engine::{js_string, Context, JsValue, NativeFunction, Source};

    use super::{ScriptAction, ScriptOutcome, ScriptSignals, SCRIPT_H, SCRIPT_W};
    use crate::params::ParamStore;

    /// Shared between the engine and the native functions on the render thread.
    #[derive(Default)]
    struct HostState {
        params: HashMap<String, f32>,
        actions: Vec<ScriptAction>,
        buffer: Vec<u8>,
        buffer_used: bool,
    }

    thread_local! {
        static HOST: RefCell<HostState> = RefCell::new(HostState::default());
    }

    pub struct ScriptEngine {
        ctx: Context,
        ready: bool,
        last_error: Option<String>,
    }

    impl Default for ScriptEngine {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ScriptEngine {
        pub fn new() -> Self {
            HOST.with(|h| h.borrow_mut().buffer = vec![0u8; SCRIPT_W * SCRIPT_H * 4]);
            let mut ctx = Context::default();
            register_api(&mut ctx);
            ScriptEngine { ctx, ready: false, last_error: None }
        }

        pub fn has_script(&self) -> bool {
            self.ready
        }

        /// Compile a new user script. The body is wrapped in a function stored as
        /// `__run` so per-frame execution is a cheap call, not a recompile.
        pub fn set_source(&mut self, src: &str) -> Result<(), String> {
            self.ready = false;
            if src.trim().is_empty() {
                // Empty script: install a no-op so per-frame calls are harmless.
                let _ = self.ctx.eval(Source::from_bytes(b"globalThis.__run=function(){};"));
                return Ok(());
            }
            // Define __run by wrapping the user body. A syntax error surfaces here.
            let wrapped = format!("globalThis.__run=function(){{\n{src}\n}};");
            match self.ctx.eval(Source::from_bytes(wrapped.as_bytes())) {
                Ok(_) => {
                    self.ready = true;
                    self.last_error = None;
                    Ok(())
                }
                Err(e) => {
                    let msg = e.to_string();
                    self.last_error = Some(msg.clone());
                    Err(msg)
                }
            }
        }

        /// Run the script for one frame. Returns the parameter writes it made,
        /// whether it drew, and any new error.
        pub fn run(&mut self, sig: &ScriptSignals, params: &ParamStore) -> ScriptOutcome {
            let mut out = ScriptOutcome::default();
            if !self.ready {
                return out;
            }

            // Snapshot the current parameter values for `get`, and reset actions.
            HOST.with(|h| {
                let mut s = h.borrow_mut();
                s.actions.clear();
                s.buffer_used = false;
                s.params.clear();
                for (id, spec, value) in params.iter() {
                    let _ = id;
                    s.params.insert(spec.path.clone(), value.as_f32());
                }
            });

            self.set_signals(sig);

            // Call __run(); a runtime error is reported (rate-limited to changes).
            if let Err(e) = self.ctx.eval(Source::from_bytes(b"globalThis.__run();")) {
                let msg = e.to_string();
                if self.last_error.as_deref() != Some(msg.as_str()) {
                    out.error = Some(msg.clone());
                }
                self.last_error = Some(msg);
            } else if self.last_error.is_some() {
                // Recovered: clear the error in the UI.
                self.last_error = None;
                out.error = Some(String::new());
            }

            HOST.with(|h| {
                let mut s = h.borrow_mut();
                out.actions = std::mem::take(&mut s.actions);
                out.buffer_used = s.buffer_used;
            });
            out
        }

        /// Copy the script's pixel buffer (RGBA, SCRIPT_W x SCRIPT_H) into `dst`.
        pub fn buffer(&self, dst: &mut Vec<u8>) {
            HOST.with(|h| {
                let s = h.borrow();
                dst.clear();
                dst.extend_from_slice(&s.buffer);
            });
        }

        fn set_signals(&mut self, sig: &ScriptSignals) {
            let g = self.ctx.global_object();
            let mut put = |name: &str, v: f64| {
                let _ = g.set(js_string!(name.to_string()), JsValue::from(v), false, &mut self.ctx);
            };
            // `put` borrows ctx mutably; build the values first.
            let pairs: [(&str, f64); 11] = [
                ("t", sig.t as f64),
                ("dt", sig.dt as f64),
                ("frame", sig.frame),
                ("low", sig.low as f64),
                ("mid", sig.mid as f64),
                ("high", sig.high as f64),
                ("rms", sig.rms as f64),
                ("onset", sig.onset as f64),
                ("beat", sig.beat as f64),
                ("bar", sig.bar as f64),
                ("bpm", sig.bpm as f64),
            ];
            drop(put);
            for (name, v) in pairs {
                let _ = self.ctx.global_object().set(js_string!(name.to_string()), JsValue::from(v), false, &mut self.ctx);
            }
            // LFOs as a JS array `lfo[0..5]`.
            let arr = boa_engine::object::builtins::JsArray::new(&mut self.ctx);
            for v in sig.lfos {
                let _ = arr.push(JsValue::from(v as f64), &mut self.ctx);
            }
            let _ = self.ctx.global_object().set(js_string!("lfo"), arr, false, &mut self.ctx);
        }
    }

    // --- native functions; all read/write the thread-local HostState ---

    fn arg_str(args: &[JsValue], i: usize, ctx: &mut Context) -> String {
        args.get(i)
            .and_then(|v| v.to_string(ctx).ok())
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default()
    }
    fn arg_num(args: &[JsValue], i: usize, ctx: &mut Context) -> f64 {
        args.get(i).and_then(|v| v.to_number(ctx).ok()).unwrap_or(0.0)
    }

    fn register_api(ctx: &mut Context) {
        macro_rules! reg {
            ($name:literal, $len:expr, $f:expr) => {
                let _ = ctx.register_global_callable(js_string!($name), $len, NativeFunction::from_fn_ptr($f));
            };
        }

        reg!("set", 2, |_t, args, ctx| {
            let p = arg_str(args, 0, ctx);
            let v = arg_num(args, 1, ctx) as f32;
            HOST.with(|h| h.borrow_mut().actions.push(ScriptAction::Set(p, v)));
            Ok(JsValue::undefined())
        });
        reg!("setn", 2, |_t, args, ctx| {
            let p = arg_str(args, 0, ctx);
            let v = arg_num(args, 1, ctx) as f32;
            HOST.with(|h| h.borrow_mut().actions.push(ScriptAction::SetNorm(p, v)));
            Ok(JsValue::undefined())
        });
        reg!("trigger", 1, |_t, args, ctx| {
            let p = arg_str(args, 0, ctx);
            HOST.with(|h| h.borrow_mut().actions.push(ScriptAction::Trigger(p)));
            Ok(JsValue::undefined())
        });
        reg!("get", 1, |_t, args, ctx| {
            let p = arg_str(args, 0, ctx);
            let v = HOST.with(|h| h.borrow().params.get(&p).copied().unwrap_or(0.0));
            Ok(JsValue::from(v as f64))
        });

        // --- 2D pixel buffer ---
        reg!("clear", 3, |_t, args, ctx| {
            let (r, g, b) = (chan(arg_num(args, 0, ctx)), chan(arg_num(args, 1, ctx)), chan(arg_num(args, 2, ctx)));
            HOST.with(|h| {
                let mut s = h.borrow_mut();
                s.buffer_used = true;
                for px in s.buffer.chunks_exact_mut(4) {
                    px[0] = r; px[1] = g; px[2] = b; px[3] = 255;
                }
            });
            Ok(JsValue::undefined())
        });
        reg!("pset", 5, |_t, args, ctx| {
            let x = arg_num(args, 0, ctx) as i32;
            let y = arg_num(args, 1, ctx) as i32;
            let (r, g, b) = (chan(arg_num(args, 2, ctx)), chan(arg_num(args, 3, ctx)), chan(arg_num(args, 4, ctx)));
            HOST.with(|h| put_px(&mut h.borrow_mut(), x, y, r, g, b));
            Ok(JsValue::undefined())
        });
        reg!("rect", 7, |_t, args, ctx| {
            let x = arg_num(args, 0, ctx) as i32;
            let y = arg_num(args, 1, ctx) as i32;
            let w = arg_num(args, 2, ctx) as i32;
            let hgt = arg_num(args, 3, ctx) as i32;
            let (r, g, b) = (chan(arg_num(args, 4, ctx)), chan(arg_num(args, 5, ctx)), chan(arg_num(args, 6, ctx)));
            HOST.with(|h| {
                let mut s = h.borrow_mut();
                for yy in y..y + hgt {
                    for xx in x..x + w {
                        put_px(&mut s, xx, yy, r, g, b);
                    }
                }
            });
            Ok(JsValue::undefined())
        });
        reg!("line", 7, |_t, args, ctx| {
            let x0 = arg_num(args, 0, ctx) as i32;
            let y0 = arg_num(args, 1, ctx) as i32;
            let x1 = arg_num(args, 2, ctx) as i32;
            let y1 = arg_num(args, 3, ctx) as i32;
            let (r, g, b) = (chan(arg_num(args, 4, ctx)), chan(arg_num(args, 5, ctx)), chan(arg_num(args, 6, ctx)));
            HOST.with(|h| draw_line(&mut h.borrow_mut(), x0, y0, x1, y1, r, g, b));
            Ok(JsValue::undefined())
        });

        // Expose the buffer size as globals SW / SH.
        let _ = ctx.global_object().set(js_string!("SW"), JsValue::from(SCRIPT_W as i32), false, ctx);
        let _ = ctx.global_object().set(js_string!("SH"), JsValue::from(SCRIPT_H as i32), false, ctx);
    }

    /// Map a 0..1 colour component to a byte (also tolerates 0..255 input).
    fn chan(v: f64) -> u8 {
        let v = if v > 1.0 { v / 255.0 } else { v };
        (v.clamp(0.0, 1.0) * 255.0) as u8
    }

    fn put_px(s: &mut HostState, x: i32, y: i32, r: u8, g: u8, b: u8) {
        if x < 0 || y < 0 || x >= SCRIPT_W as i32 || y >= SCRIPT_H as i32 {
            return;
        }
        s.buffer_used = true;
        let i = (y as usize * SCRIPT_W + x as usize) * 4;
        s.buffer[i] = r;
        s.buffer[i + 1] = g;
        s.buffer[i + 2] = b;
        s.buffer[i + 3] = 255;
    }

    fn draw_line(s: &mut HostState, mut x0: i32, mut y0: i32, x1: i32, y1: i32, r: u8, g: u8, b: u8) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            put_px(s, x0, y0, r, g, b);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }
}

/// No-op stand-in used when the `script` feature is disabled.
#[cfg(not(feature = "script"))]
mod stub {
    use super::{ScriptOutcome, ScriptSignals};
    use crate::params::ParamStore;

    #[derive(Default)]
    pub struct ScriptEngine;

    impl ScriptEngine {
        pub fn new() -> Self {
            ScriptEngine
        }
        pub fn has_script(&self) -> bool {
            false
        }
        pub fn set_source(&mut self, _src: &str) -> Result<(), String> {
            Err("scripting not built in (enable the `script` feature)".into())
        }
        pub fn run(&mut self, _sig: &ScriptSignals, _params: &ParamStore) -> ScriptOutcome {
            ScriptOutcome::default()
        }
        pub fn buffer(&self, _dst: &mut Vec<u8>) {}
    }
}
