//! The Z-Axis desktop app: layout, chat/agent runs, version viewer & compare, map windows, region capture.

use crate::agent::{self, Agent, Event};
use crate::glb::{MapKind, Model};
use crate::select::{self, View};
use crate::sys::d3d::{Frame, Renderer};
use crate::sys::win::{self, Canvas, Ctl, Mods, Mouse, Msg, Wnd, rgb};
use crate::{blender, skills};
use glam::{Mat4, Vec2, Vec3};
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};

// Control ids
const ID_AGENT: u16 = 100;
const ID_MODEL: u16 = 101;
const ID_SEND: u16 = 102;
const ID_STOP: u16 = 103;
const ID_CAPTURE: u16 = 104;
const ID_CLEAR: u16 = 105;
const ID_VERSIONS: u16 = 106;
const ID_REFS: u16 = 107;
const ID_ADD_REF: u16 = 108;
const ID_INPUT: u16 = 109;
// Menu ids
const M_OPEN_WS: u16 = 200;
const M_NEW_CHAT: u16 = 201;
const M_INSTALL_ADDON: u16 = 202;
const M_LAUNCH_BLENDER: u16 = 203;
const M_EXIT: u16 = 204;
const M_WIREFRAME: u16 = 210;
const M_COMPARE: u16 = 211;
const M_FRAME: u16 = 212;
const M_MAP: u16 = 220; // + MapKind index

const TIMER_POLL: usize = 1;
const LBN_SELCHANGE: u16 = 1;
const CBN_SELCHANGE: u16 = 1;

const ADDON_INIT: &str = include_str!("../blender_addon/zaxis_bridge/__init__.py");
const ADDON_MANIFEST: &str = include_str!("../blender_addon/zaxis_bridge/blender_manifest.toml");

/// Results from worker threads, delivered to the UI thread through `Poster`.
enum Payload {
    Agent(Event),
    AgentExit { ok: bool, stderr: String },
    Region(Result<serde_json::Value, String>, PathBuf),
    Info(String),
}

#[derive(Clone, Copy, PartialEq)]
enum CaptureMode {
    Off,
    Box,
    Lasso,
}

struct Camera {
    target: Vec3,
    dist: f32,
    yaw: f32,
    pitch: f32,
}

impl Camera {
    fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cp * sy, sp, cp * cy) * self.dist
    }
    fn view(&self, size: Vec2) -> View {
        let (near, far) = (self.dist * 0.01, self.dist * 100.0);
        let proj = Mat4::perspective_rh(0.8, size.x.max(1.0) / size.y.max(1.0), near, far);
        View { view_proj: proj * Mat4::look_at_rh(self.eye(), self.target, Vec3::Y), eye: self.eye(), near, far, size }
    }
    fn frame(&mut self, m: &Model) {
        self.target = (m.min + m.max) * 0.5;
        self.dist = ((m.max - m.min).length() * 1.2).max(0.1);
    }
}

struct MapWin {
    kind: MapKind,
    wnd: Wnd,
    material: usize,
}

struct Loaded {
    path: PathBuf,
    model: Model,
}

struct App {
    ws: PathBuf,
    exe: PathBuf,
    main: Wnd,
    view: Wnd,
    chat: Wnd,
    input: Wnd,
    send: Wnd,
    stop: Wnd,
    agent_box: Wnd,
    model_box: Wnd,
    capture_btn: Wnd,
    clear_btn: Wnd,
    versions: Wnd,
    refs: Wnd,
    add_ref: Wnd,
    labels: [Wnd; 3],
    status: Wnd,
    renderer: Option<Renderer>,
    a: Option<Loaded>,
    b: Option<Loaded>,
    compare: bool,
    wireframe: bool,
    cam: Camera,
    drag: Option<(Mouse, i32, i32)>,
    capture: CaptureMode,
    outline: Vec<Vec2>,
    selection: Vec<Vec<u32>>,
    region: Option<String>,
    agent: Agent,
    sessions: HashMap<Agent, String>,
    child: Option<Arc<Mutex<Child>>>,
    agent_talking: bool,
    version_files: Vec<PathBuf>,
    ref_files: Vec<PathBuf>,
    maps: Vec<MapWin>,
    blender_up: bool,
}

pub fn run(ws: PathBuf) {
    if let Err(e) = win::run(|| build(ws).map(|a| Box::new(a) as Box<dyn win::Handler>)) {
        win::log(&e);
    }
}

fn build(ws: PathBuf) -> Result<App, win::Error> {
    let main = Wnd::top("Z-Axis", 1500, 900, false)?;
    main.set_menu(&[
        ("&File", &[(M_OPEN_WS, "Open workspace folder"), (M_NEW_CHAT, "New chat"), (0, ""), (M_INSTALL_ADDON, "Install Blender add-on"), (M_LAUNCH_BLENDER, "Launch Blender"), (0, ""), (M_EXIT, "Exit")]),
        (
            "&View",
            &[
                (M_FRAME, "Frame model\tF"),
                (M_WIREFRAME, "Wireframe"),
                (M_COMPARE, "Compare versions"),
                (0, ""),
                (M_MAP, "Base Color map"),
                (M_MAP + 1, "Normal map"),
                (M_MAP + 2, "Metallic / Roughness map"),
                (M_MAP + 3, "Ambient Occlusion map"),
                (M_MAP + 4, "Emissive map"),
                (M_MAP + 5, "UV layout"),
            ],
        ),
    ])?;
    let agent_box = main.child(Ctl::Combo, ID_AGENT, "")?;
    let installed: Vec<bool> = Agent::ALL.iter().map(|a| a.find_exe().is_some()).collect();
    for (a, ok) in Agent::ALL.iter().zip(&installed) {
        agent_box.combo_add(&format!("{}{}", a.label(), if *ok { "" } else { "  (not installed)" }));
    }
    let agent = Agent::ALL.iter().zip(&installed).find(|(_, ok)| **ok).map(|(a, _)| *a).unwrap_or(Agent::Claude);
    agent_box.combo_set_sel(Agent::ALL.iter().position(|a| *a == agent).unwrap_or(0));

    let view = main.child(Ctl::View, 0, "")?;
    let chat = main.child(Ctl::Edit { readonly: true }, 0, "")?;
    let input = main.child(Ctl::Edit { readonly: false }, ID_INPUT, "")?;
    win::enter_sends(input, ID_SEND);
    let mut app = App {
        exe: std::env::current_exe().map_err(|e| e.to_string())?,
        main,
        view,
        chat,
        input,
        send: main.child(Ctl::Button, ID_SEND, "Send")?,
        stop: main.child(Ctl::Button, ID_STOP, "Stop")?,
        agent_box,
        model_box: main.child(Ctl::Line, ID_MODEL, "")?,
        capture_btn: main.child(Ctl::Button, ID_CAPTURE, "Capture: Off")?,
        clear_btn: main.child(Ctl::Button, ID_CLEAR, "Clear region")?,
        versions: main.child(Ctl::List { multi: false }, ID_VERSIONS, "")?,
        refs: main.child(Ctl::List { multi: true }, ID_REFS, "")?,
        add_ref: main.child(Ctl::Button, ID_ADD_REF, "Add reference…")?,
        labels: [main.child(Ctl::Static, 0, "Versions")?, main.child(Ctl::Static, 0, "References (selected = attached)")?, main.child(Ctl::Static, 0, "Model:")?],
        status: main.child(Ctl::Static, 0, "")?,
        renderer: None,
        a: None,
        b: None,
        compare: false,
        wireframe: false,
        cam: Camera { target: Vec3::ZERO, dist: 6.0, yaw: 0.6, pitch: 0.4 },
        drag: None,
        capture: CaptureMode::Off,
        outline: vec![],
        selection: vec![],
        region: None,
        agent,
        sessions: HashMap::new(),
        child: None,
        agent_talking: false,
        version_files: vec![],
        ref_files: vec![],
        maps: vec![],
        blender_up: false,
        ws,
    };
    app.stop.enable(false);
    app.renderer = match Renderer::new(view) {
        Ok(r) => Some(r),
        Err(e) => {
            app.say(&format!("[viewer] {e}\n"));
            None
        }
    };
    app.select_agent(agent);
    app.say("Z-Axis ready. Start Blender with the Z-Axis Bridge add-on (File > Install Blender add-on), then type an instruction.\nEnter sends, Shift+Enter adds a line.\n");
    main.accept_files();
    main.set_timer(TIMER_POLL, 1000);
    app.poll();
    app.refresh_refs();
    app.layout();
    main.show(true);
    Ok(app)
}

impl win::Handler for App {
    fn on(&mut self, w: Wnd, msg: Msg) -> bool {
        if let Some(i) = self.maps.iter().position(|m| m.wnd == w) {
            return self.on_map(i, msg);
        }
        match msg {
            Msg::Size if w == self.main => self.layout(),
            Msg::Paint(c) if w == self.view => {
                c.discard();
                self.render();
            }
            Msg::Mouse { kind, x, y, mods } if w == self.view => self.on_mouse(kind, x, y, mods),
            Msg::Wheel(d) => {
                self.cam.dist *= 0.85f32.powf(d);
                self.view.invalidate();
            }
            Msg::Key(k) if w == self.view && k == 'F' as u32 => self.frame(),
            Msg::Command { id, code } => self.on_command(id, code),
            Msg::Timer(TIMER_POLL) => self.poll(),
            Msg::Drop(files) => self.add_refs(files),
            Msg::Payload(p) => {
                if let Ok(p) = p.downcast::<Payload>() {
                    self.on_payload(*p);
                }
            }
            Msg::Close if w == self.main => {
                self.kill_agent();
                win::quit();
                return false;
            }
            _ => return false,
        }
        true
    }
}

impl App {
    fn say(&self, s: &str) {
        self.chat.append(s);
    }

    fn layout(&mut self) {
        let s = self.main.dpi();
        let px = |v: f32| (v * s) as i32;
        let (w, h) = self.main.client_size();
        let (pad, bar, left, right, status) = (px(6.), px(30.), px(210.), px(420.), px(24.));
        // toolbar
        self.agent_box.set_pos(pad, pad, px(250.), px(300.));
        self.labels[2].set_pos(pad * 2 + px(250.), pad, px(46.), bar - pad);
        self.model_box.set_pos(pad * 2 + px(296.), pad, px(170.), bar - pad);
        self.capture_btn.set_pos(pad * 3 + px(466.), pad, px(130.), bar - pad);
        self.clear_btn.set_pos(pad * 4 + px(596.), pad, px(110.), bar - pad);
        let top = bar + pad;
        let body = h - top - status - pad;
        // left column
        let half = (body - px(22.) * 2 - bar - pad) / 2;
        self.labels[0].set_pos(pad, top, left, px(20.));
        self.versions.set_pos(pad, top + px(22.), left, half);
        self.labels[1].set_pos(pad, top + px(22.) + half + pad, left, px(20.));
        self.refs.set_pos(pad, top + px(44.) + half + pad, left, half - pad);
        self.add_ref.set_pos(pad, top + body - bar + pad, left, bar - pad);
        // right column
        let rx = w - right - pad;
        let input_h = px(90.);
        self.chat.set_pos(rx, top, right, body - input_h - bar - pad);
        self.input.set_pos(rx, top + body - input_h - bar, right, input_h);
        self.send.set_pos(rx, top + body - bar + pad, right / 2 - pad / 2, bar - pad);
        self.stop.set_pos(rx + right / 2 + pad / 2, top + body - bar + pad, right / 2 - pad / 2, bar - pad);
        // viewport + status
        let vx = left + pad * 2;
        let (vw, vh) = (rx - pad - vx, body);
        self.view.set_pos(vx, top, vw, vh);
        self.status.set_pos(pad, h - status, w - pad * 2, status);
        // Our own view's WM_SIZE is swallowed while we're inside the handler, so resize here.
        if let Some(r) = &mut self.renderer
            && let Err(e) = r.resize(vw, vh)
        {
            win::log(&e);
        }
        self.view.invalidate();
    }

    // ------------------------------------------------------------ viewport

    fn view_size(&self) -> Vec2 {
        let (w, h) = self.view.client_size();
        Vec2::new(if self.compare { w as f32 / 2.0 } else { w as f32 }, h as f32)
    }

    fn render(&self) {
        let Some(r) = &self.renderer else { return };
        let v = self.cam.view(self.view_size());
        r.render(&Frame { view_proj: v.view_proj, eye: v.eye, compare: self.compare, wireframe: self.wireframe, outline: &self.outline });
    }

    fn frame(&mut self) {
        if let Some(a) = &self.a {
            self.cam.frame(&a.model);
        }
        self.view.invalidate();
    }

    fn on_mouse(&mut self, kind: Mouse, x: i32, y: i32, mods: Mods) {
        let p = Vec2::new(x as f32, y as f32);
        let capturing = self.capture != CaptureMode::Off && !self.compare;
        match kind {
            Mouse::LDown | Mouse::RDown | Mouse::MDown => {
                self.view.focus();
                self.view.capture(true);
                self.drag = Some((kind, x, y));
                if capturing && kind == Mouse::LDown {
                    self.outline = vec![p];
                }
            }
            Mouse::Move => {
                let Some((button, lx, ly)) = self.drag else { return };
                let (dx, dy) = ((x - lx) as f32, (y - ly) as f32);
                if capturing && button == Mouse::LDown {
                    match self.capture {
                        CaptureMode::Box => self.outline = select::rect(self.outline[0], p),
                        _ => self.outline.push(p),
                    }
                } else if button == Mouse::LDown && !mods.shift {
                    self.cam.yaw -= dx * 0.008;
                    self.cam.pitch = (self.cam.pitch + dy * 0.008).clamp(-1.5, 1.5);
                } else {
                    // Pan: right/middle drag, or Shift+left drag.
                    let v = self.cam.view(self.view_size());
                    let inv = v.view_proj.inverse();
                    let right = (inv * glam::Vec4::new(1.0, 0.0, 0.0, 0.0)).truncate().normalize_or_zero();
                    let up = (inv * glam::Vec4::new(0.0, 1.0, 0.0, 0.0)).truncate().normalize_or_zero();
                    let k = self.cam.dist * 0.0015;
                    self.cam.target += (-right * dx + up * dy) * k;
                }
                self.drag = Some((button, x, y));
                self.view.invalidate();
            }
            Mouse::LUp | Mouse::RUp | Mouse::MUp => {
                self.view.capture(false);
                let was = self.drag.take();
                if capturing && matches!(was, Some((Mouse::LDown, ..))) {
                    let outline = std::mem::take(&mut self.outline);
                    if outline.len() >= 3 {
                        self.finish_capture(&outline, mods);
                    }
                    self.view.invalidate();
                }
            }
        }
    }

    // ------------------------------------------------------------ region capture

    fn finish_capture(&mut self, outline: &[Vec2], mods: Mods) {
        let (Some(r), Some(a)) = (&self.renderer, &self.a) else { return };
        let view = self.cam.view(self.view_size());
        let cap = match r.capture(&Frame { view_proj: view.view_proj, eye: view.eye, compare: false, wireframe: false, outline: &[] }) {
            Ok(c) => c,
            Err(e) => return self.say(&format!("[capture] {e}\n")),
        };
        let depth = |p: Vec2| {
            let (x, y) = (p.x as i64, p.y as i64);
            ((0..cap.w as i64).contains(&x) && (0..cap.h as i64).contains(&y)).then(|| cap.depth[(y * cap.w as i64 + x) as usize])
        };
        let picked = select::pick(&a.model.meshes, &view, outline, &depth);
        self.selection.resize(a.model.meshes.len(), vec![]);
        for (cur, new) in self.selection.iter_mut().zip(picked) {
            if mods.ctrl {
                cur.retain(|t| !new.contains(t));
            } else if mods.shift {
                cur.extend(new.into_iter().filter(|t| !cur.contains(t)).collect::<Vec<_>>());
            } else {
                *cur = new;
            }
        }
        if let Some(r) = &mut self.renderer
            && let Err(e) = r.set_selection(&self.selection)
        {
            win::log(&e);
        }

        // Region image: crop of what the user saw, padded a bit.
        let (lo, hi) = outline.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let lo = (lo - 24.0).max(Vec2::ZERO);
        let hi = (hi + 24.0).min(Vec2::new(cap.w as f32, cap.h as f32));
        let image_path = self.ws.join("region.png");
        if let Some(img) = image::RgbaImage::from_raw(cap.w, cap.h, cap.rgba) {
            let crop = image::imageops::crop_imm(&img, lo.x as u32, lo.y as u32, (hi.x - lo.x) as u32, (hi.y - lo.y) as u32).to_image();
            if let Err(e) = crop.save(&image_path) {
                win::log(&e.to_string());
            }
        }

        // World-space (glTF) centres of selected triangles, grouped per Blender object.
        let mut by_object: Vec<(String, Vec<[f32; 3]>)> = vec![];
        for (mesh, tris) in a.model.meshes.iter().zip(&self.selection) {
            if tris.is_empty() {
                continue;
            }
            let pts = tris.iter().map(|&t| {
                let [p, q, r] = mesh.tri(t as usize);
                ((p + q + r) / 3.0).into()
            });
            match by_object.iter_mut().find(|(n, _)| *n == mesh.name) {
                Some((_, v)) => v.extend(pts),
                None => by_object.push((mesh.name.clone(), pts.collect())),
            }
        }
        if by_object.is_empty() {
            return self.clear_region();
        }
        let regions: Vec<_> = by_object.iter().map(|(o, p)| json!({"object": o, "points": p})).collect();
        let poster = self.main.poster();
        std::thread::spawn(move || {
            let r = blender::call("select_region", json!({"regions": regions, "space": "gltf"}));
            poster.post(Box::new(Payload::Region(r, image_path)));
        });
        self.view.invalidate();
    }

    fn clear_region(&mut self) {
        self.selection.clear();
        self.region = None;
        let _ = std::fs::remove_file(self.ws.join("region.json"));
        if let Some(r) = &mut self.renderer {
            let _ = r.set_selection(&[]);
        }
        self.update_status();
        self.view.invalidate();
    }

    // ------------------------------------------------------------ commands

    fn on_command(&mut self, id: u16, code: u16) {
        match id {
            ID_SEND => self.send_prompt(),
            ID_STOP => self.kill_agent(),
            ID_AGENT if code == CBN_SELCHANGE => {
                if let Some(i) = self.agent_box.combo_sel() {
                    self.select_agent(Agent::ALL[i]);
                }
            }
            ID_CAPTURE => {
                self.capture = match self.capture {
                    CaptureMode::Off => CaptureMode::Lasso,
                    CaptureMode::Lasso => CaptureMode::Box,
                    CaptureMode::Box => CaptureMode::Off,
                };
                self.capture_btn.set_text(match self.capture {
                    CaptureMode::Off => "Capture: Off",
                    CaptureMode::Lasso => "Capture: Lasso",
                    CaptureMode::Box => "Capture: Box",
                });
                if self.capture != CaptureMode::Off {
                    self.say("[capture] Drag over the model. Shift = add, Ctrl = remove. Right/middle drag still moves the camera.\n");
                }
            }
            ID_CLEAR => self.clear_region(),
            ID_VERSIONS if code == LBN_SELCHANGE => {
                if let Some(path) = self.versions.list_sel().and_then(|i| self.version_files.get(i).cloned()) {
                    self.load(if self.compare { 1 } else { 0 }, &path);
                }
            }
            ID_ADD_REF => {
                let files = self.main.open_images();
                self.add_refs(files);
            }
            M_OPEN_WS => {
                let _ = std::process::Command::new("explorer").arg(&self.ws).spawn();
            }
            M_NEW_CHAT => {
                self.sessions.remove(&self.agent);
                self.chat.set_text("");
                self.say(&format!("New chat with {}.\n", self.agent.label()));
            }
            M_INSTALL_ADDON => self.install_addon(),
            M_LAUNCH_BLENDER => match blender_exes().pop() {
                Some(b) => {
                    let _ = std::process::Command::new(b).spawn();
                }
                None => self.say("[blender] No Blender 5.x found in Program Files.\n"),
            },
            M_EXIT => {
                self.kill_agent();
                win::quit();
            }
            M_WIREFRAME => {
                self.wireframe = !self.wireframe;
                self.main.check_menu(M_WIREFRAME, self.wireframe);
                self.view.invalidate();
            }
            M_FRAME => self.frame(),
            M_COMPARE => self.toggle_compare(),
            id if (M_MAP..M_MAP + 6).contains(&id) => self.toggle_map(MapKind::ALL[(id - M_MAP) as usize]),
            _ => {}
        }
    }

    fn select_agent(&mut self, agent: Agent) {
        self.agent = agent;
        if let Err(e) = skills::install(agent, &self.ws, &self.exe) {
            self.say(&format!("[skills] could not install for {}: {e}\n", agent.label()));
        }
        let model = agent.default_model();
        self.model_box.set_text(model);
        if agent.find_exe().is_none() {
            let hint = match agent {
                Agent::Claude => "npm i -g @anthropic-ai/claude-code",
                Agent::Gemini => "npm i -g @google/gemini-cli",
                _ => "npm i -g @openai/codex",
            };
            self.say(&format!("[agent] {} is not installed. Install with: {hint}\n", agent.label()));
        }
        self.update_status();
    }

    // ------------------------------------------------------------ agent runs

    fn send_prompt(&mut self) {
        let text = self.input.text().trim().to_string();
        if text.is_empty() || self.child.is_some() {
            return;
        }
        let Some(cli) = self.agent.find_exe() else {
            return self.say(&format!("[agent] {} is not installed.\n", self.agent.label()));
        };
        if let Err(e) = skills::install(self.agent, &self.ws, &self.exe) {
            return self.say(&format!("[skills] {e}\n"));
        }
        let refs: Vec<PathBuf> = self.refs.list_multi_sel().into_iter().filter_map(|i| self.ref_files.get(i).cloned()).collect();
        let prompt = agent::prompt(self.agent, &text, self.region.as_deref(), &refs);
        let model = self.model_box.text();
        let mut cmd = agent::command(&agent::Run {
            agent: self.agent,
            cli: &cli,
            model: model.trim(),
            session: self.sessions.get(&self.agent).map(String::as_str),
            images: &refs,
            workspace: &self.ws,
            zaxis_exe: &self.exe,
        });
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return self.say(&format!("[agent] failed to start {}: {e}\n", cli.display())),
        };
        self.input.set_text("");
        self.say(&format!("\nYou: {text}\n"));
        self.agent_talking = false;

        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let child = Arc::new(Mutex::new(child));
        self.child = Some(child.clone());
        self.send.enable(false);
        self.stop.enable(true);
        self.update_status();

        let agent = self.agent;
        let poster = self.main.poster();
        std::thread::spawn(move || {
            if let Some(mut i) = stdin {
                let _ = i.write_all(prompt.as_bytes());
            } // dropped: closes stdin so the CLI starts
            let err_thread = std::thread::spawn(move || {
                let mut s = String::new();
                if let Some(mut e) = stderr {
                    let _ = e.read_to_string(&mut s);
                }
                s
            });
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    for ev in agent::parse(agent, &line) {
                        poster.post(Box::new(Payload::Agent(ev)));
                    }
                }
            }
            let ok = child.lock().map(|mut c| c.wait().is_ok_and(|s| s.success())).unwrap_or(false);
            let stderr = err_thread.join().unwrap_or_default();
            poster.post(Box::new(Payload::AgentExit { ok, stderr }));
        });
    }

    fn kill_agent(&mut self) {
        if let Some(c) = &self.child
            && let Ok(mut c) = c.try_lock()
        {
            let _ = c.kill();
        }
    }

    fn on_payload(&mut self, p: Payload) {
        match p {
            Payload::Agent(ev) => match ev {
                Event::Text(t) => {
                    if !self.agent_talking {
                        self.say(&format!("{}: ", self.agent.label()));
                        self.agent_talking = true;
                    }
                    self.say(&t);
                }
                Event::Tool(name) => {
                    self.say(&format!("   ⚙ {name}\n"));
                    self.agent_talking = false;
                }
                Event::Session(id) if !id.is_empty() => {
                    self.sessions.insert(self.agent, id);
                }
                Event::Error(e) => self.say(&format!("[error] {e}\n")),
                _ => {}
            },
            Payload::AgentExit { ok, stderr } => {
                self.child = None;
                self.send.enable(true);
                self.stop.enable(false);
                if !ok {
                    let tail: String = stderr.chars().rev().take(1200).collect::<Vec<_>>().into_iter().rev().collect();
                    self.say(&format!("[agent stopped]{}\n", if tail.trim().is_empty() { String::new() } else { format!("\n{}", tail.trim()) }));
                }
                self.poll();
                self.update_status();
            }
            Payload::Region(r, image) => match r {
                Ok(v) => {
                    let faces: usize = v["regions"].as_array().into_iter().flatten().map(|r| r["faces"].as_array().map_or(0, Vec::len)).sum();
                    let objects: Vec<&str> = v["regions"].as_array().into_iter().flatten().filter_map(|r| r["object"].as_str()).collect();
                    let summary = format!("{} on {} ({faces} Blender faces), crop image {}", v["group"].as_str().unwrap_or("zaxis_region"), objects.join(", "), image.display());
                    let mut doc = v.clone();
                    doc["image"] = json!(image);
                    let _ = std::fs::write(self.ws.join("region.json"), serde_json::to_string_pretty(&doc).unwrap_or_default());
                    self.say(&format!("[region] {summary}. Your next prompt applies to this region.\n"));
                    self.region = Some(summary);
                    self.update_status();
                }
                Err(e) => self.say(&format!("[region] Blender could not select the region: {e}\n")),
            },
            Payload::Info(s) => self.say(&s),
        }
    }

    // ------------------------------------------------------------ versions & compare

    fn poll(&mut self) {
        let up = std::net::TcpStream::connect_timeout(&blender::ADDR.parse().expect("const addr"), std::time::Duration::from_millis(150)).is_ok();
        let changed_up = up != self.blender_up;
        self.blender_up = up;

        // shortcut: 1 s directory polling; switch to ReadDirectoryChangesW if exports ever feel laggy.
        let dir = self.ws.join("versions");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            // The .json sidecar is written after the GLB, so its presence means the GLB is complete.
            .filter(|p| p.extension().is_some_and(|e| e == "glb") && p.with_extension("json").exists())
            .collect();
        files.sort();
        if files != self.version_files {
            let newest = files.last().cloned();
            let had_newest = newest.as_ref().is_some_and(|n| self.version_files.contains(n));
            self.version_files = files;
            self.versions.list_clear();
            for f in &self.version_files {
                let note = std::fs::read_to_string(f.with_extension("json")).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).and_then(|v| v["note"].as_str().map(str::to_string)).unwrap_or_default();
                let stem = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                self.versions.list_add(&if note.is_empty() { stem } else { format!("{stem}  {note}") });
            }
            if let Some(n) = newest
                && !had_newest
                && !self.compare
            {
                self.load(0, &n);
            }
            self.select_loaded_in_list();
        } else if changed_up {
            self.update_status();
        }
    }

    fn select_loaded_in_list(&self) {
        let cur = if self.compare { &self.b } else { &self.a };
        if let Some(i) = cur.as_ref().and_then(|l| self.version_files.iter().position(|f| *f == l.path)) {
            self.versions.list_set_sel(i);
        }
    }

    fn load(&mut self, slot: usize, path: &Path) {
        let model = match Model::load(path) {
            Ok(m) => m,
            Err(e) => return self.say(&format!("[viewer] {e}\n")),
        };
        if let Some(r) = &mut self.renderer
            && let Err(e) = r.set_model(slot, Some(&model))
        {
            self.say(&format!("[viewer] {e}\n"));
        }
        let first = self.a.is_none();
        let loaded = Some(Loaded { path: path.to_path_buf(), model });
        if slot == 0 {
            self.a = loaded;
            self.selection.clear(); // triangle indices don't carry over; the Blender-side group does
            if first {
                self.frame();
            }
        } else {
            self.b = loaded;
        }
        for m in &self.maps {
            m.wnd.invalidate();
        }
        self.update_status();
        self.view.invalidate();
    }

    fn toggle_compare(&mut self) {
        if self.a.is_none() {
            return self.say("[compare] Load a version first.\n");
        }
        self.compare = !self.compare;
        self.main.check_menu(M_COMPARE, self.compare);
        if self.compare {
            // Right side starts at the previous version; pick another in the list to change it.
            let cur = self.a.as_ref().and_then(|a| self.version_files.iter().position(|f| *f == a.path)).unwrap_or(0);
            let other = self.version_files.get(cur.saturating_sub(1)).cloned();
            if let Some(o) = other {
                self.load(1, &o);
            }
            self.say("[compare] Left = pinned version, right = list selection. Camera is shared.\n");
        } else {
            self.b = None;
            if let Some(r) = &mut self.renderer {
                let _ = r.set_model(1, None);
            }
        }
        self.select_loaded_in_list();
        self.update_status();
        self.view.invalidate();
    }

    fn update_status(&self) {
        let name = |l: &Loaded| l.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let stats = |l: &Loaded| {
            let s = l.model.stats();
            format!("{}: {} tris · {} verts · {} obj · {} mat · {:.2}×{:.2}×{:.2} m", name(l), s.tris, s.verts, s.objects, s.materials, s.size.x, s.size.y, s.size.z)
        };
        let mut parts = vec![
            format!("Blender: {}", if self.blender_up { "connected" } else { "not running" }),
            format!("{}: {}", self.agent.label(), if self.child.is_some() { "working…" } else { "idle" }),
        ];
        match (&self.a, &self.b) {
            (Some(a), Some(b)) if self.compare => {
                let (sa, sb) = (a.model.stats(), b.model.stats());
                parts.push(format!("L {}", stats(a)));
                parts.push(format!("R {}", stats(b)));
                parts.push(format!("Δ {:+} tris, {:+} verts", sb.tris as i64 - sa.tris as i64, sb.verts as i64 - sa.verts as i64));
            }
            (Some(a), _) => parts.push(stats(a)),
            _ => parts.push("No export yet".into()),
        }
        if self.region.is_some() {
            parts.push("Region active".into());
        }
        self.status.set_text(&parts.join("   |   "));
    }

    // ------------------------------------------------------------ references

    fn add_refs(&mut self, files: Vec<PathBuf>) {
        let dir = self.ws.join("references");
        let _ = std::fs::create_dir_all(&dir);
        for f in files {
            let Some(name) = f.file_name() else { continue };
            if let Err(e) = std::fs::copy(&f, dir.join(name)) {
                self.say(&format!("[references] {}: {e}\n", f.display()));
            }
        }
        self.refresh_refs();
    }

    fn refresh_refs(&mut self) {
        let keep: Vec<PathBuf> = self.refs.list_multi_sel().into_iter().filter_map(|i| self.ref_files.get(i).cloned()).collect();
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.ws.join("references")).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file()).collect();
        files.sort();
        self.refs.list_clear();
        for (i, f) in files.iter().enumerate() {
            self.refs.list_add(&f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            if keep.contains(f) {
                self.refs.list_set_multi(i, true);
            }
        }
        self.ref_files = files;
    }

    // ------------------------------------------------------------ map windows

    fn toggle_map(&mut self, kind: MapKind) {
        let id = M_MAP + kind as u16;
        if let Some(m) = self.maps.iter().find(|m| m.kind == kind) {
            let on = !m.wnd.visible();
            m.wnd.show(on);
            self.main.check_menu(id, on);
            return;
        }
        match Wnd::top(kind.label(), 560, 600, true) {
            Ok(w) => {
                w.show(true);
                self.maps.push(MapWin { kind, wnd: w, material: 0 });
                self.main.check_menu(id, true);
            }
            Err(e) => self.say(&format!("[maps] {e}\n")),
        }
    }

    fn on_map(&mut self, i: usize, msg: Msg) -> bool {
        match msg {
            Msg::Paint(c) => self.paint_map(&self.maps[i], c),
            Msg::Size => self.maps[i].wnd.invalidate(),
            Msg::Mouse { kind: Mouse::LDown, .. } => {
                let n = self.a.as_ref().map_or(1, |a| a.model.materials.len().max(1));
                self.maps[i].material = (self.maps[i].material + 1) % n;
                self.maps[i].wnd.invalidate();
            }
            Msg::Close => {
                self.maps[i].wnd.show(false);
                self.main.check_menu(M_MAP + self.maps[i].kind as u16, false);
            }
            _ => return false,
        }
        true
    }

    fn paint_map(&self, m: &MapWin, c: &mut Canvas) {
        let (w, h) = (c.w, c.h);
        c.fill(0, 0, w, h, rgb(30, 30, 34));
        let Some(a) = &self.a else {
            return c.text(12, 12, "No model loaded.", rgb(200, 200, 200));
        };
        let model = &a.model;
        let Some(mat) = model.materials.get(m.material) else {
            return c.text(12, 12, "Model has no materials.", rgb(200, 200, 200));
        };
        let header = 32;
        c.text(12, 8, &format!("{}  ({}/{})  — click to switch material", mat.name, m.material + 1, model.materials.len()), rgb(230, 230, 230));
        let size = (w - 24).min(h - header - 12).max(16);
        let (ox, oy) = ((w - size) / 2, header + (h - header - size) / 2);
        match mat.map(m.kind).and_then(|i| model.images.get(i)) {
            Some(img) => {
                // Keep aspect: fit the image inside the square.
                let (iw, ih) = (img.width as f32, img.height as f32);
                let k = size as f32 / iw.max(ih);
                let (dw, dh) = ((iw * k) as i32, (ih * k) as i32);
                c.image(img, ox + (size - dw) / 2, oy + (size - dh) / 2, dw, dh);
            }
            None => {
                c.fill(ox, oy, size, size, rgb(45, 45, 50));
                let col = mat.base_color.map(|v| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8);
                if m.kind == MapKind::BaseColor {
                    c.fill(ox, oy, size, size, rgb(col[0], col[1], col[2]));
                }
                c.text(ox + 8, oy + 8, &format!("No {} texture in this material", m.kind.label()), rgb(220, 220, 220));
            }
        }
        if m.kind == MapKind::Uv {
            let s = size as f32;
            let mut segs = vec![];
            for mesh in model.meshes.iter().filter(|me| me.material == Some(m.material) && !me.uvs.is_empty()) {
                for t in mesh.indices.chunks(3) {
                    let uv = |i: u32| mesh.uvs[i as usize] * s + Vec2::new(ox as f32, oy as f32);
                    let (p, q, r) = (uv(t[0]), uv(t[1]), uv(t[2]));
                    segs.extend([(p.x, p.y, q.x, q.y), (q.x, q.y, r.x, r.y), (r.x, r.y, p.x, p.y)]);
                }
            }
            c.lines(&segs, rgb(255, 190, 40));
        }
    }

    // ------------------------------------------------------------ Blender add-on install

    fn install_addon(&self) {
        let exes = blender_exes();
        if exes.is_empty() {
            return self.say("[blender] No Blender 5.x found under Program Files\\Blender Foundation.\n");
        }
        self.say("[blender] Installing Z-Axis Bridge…\n");
        let poster = self.main.poster();
        std::thread::spawn(move || {
            let appdata = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
            for exe in exes {
                let ver = exe.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().replace("Blender ", "")).unwrap_or_default();
                let dir = appdata.join("Blender Foundation").join("Blender").join(&ver).join("extensions").join("user_default").join("zaxis_bridge");
                let res = std::fs::create_dir_all(&dir)
                    .and_then(|_| std::fs::write(dir.join("__init__.py"), ADDON_INIT))
                    .and_then(|_| std::fs::write(dir.join("blender_manifest.toml"), ADDON_MANIFEST))
                    .map_err(|e| e.to_string())
                    .and_then(|_| {
                        let out = std::process::Command::new(&exe)
                            .args(["--background", "--python-expr"]) // no --factory-startup: save_userpref would wipe the user's prefs
                            .arg("import bpy; bpy.ops.preferences.addon_enable(module='bl_ext.user_default.zaxis_bridge'); bpy.ops.wm.save_userpref()")
                            .output()
                            .map_err(|e| e.to_string())?;
                        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).to_string()) }
                    });
                let msg = match res {
                    Ok(()) => format!("[blender] Installed and enabled in Blender {ver}. Restart Blender if it's open.\n"),
                    Err(e) => format!("[blender] Blender {ver}: {e}\n"),
                };
                poster.post(Box::new(Payload::Info(msg)));
            }
        });
    }
}

/// Installed Blender 5.x executables, oldest first.
fn blender_exes() -> Vec<PathBuf> {
    let root = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files")).join("Blender Foundation");
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Blender 5")))
        .map(|p| p.join("blender.exe"))
        .filter(|p| p.exists())
        .collect();
    v.sort();
    v
}
