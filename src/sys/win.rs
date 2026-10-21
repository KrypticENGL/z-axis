//! Safe wrappers over Win32 windows, controls, menus, GDI painting and the message loop.
//!
//! One `Handler` (the app) receives messages for every window we create; it tells them apart by `Wnd`.
//! Messages that arrive while the handler is already running (re-entrancy from SetWindowPos, modal
//! dialogs...) go to DefWindowProcW, except `WM_APP` payloads, which are queued and delivered later.

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

pub type Error = String;

/// `WM_APP` carries a `Box<Box<dyn Any + Send>>` in LPARAM, created by `Poster::post`, freed in `wndproc`.
const WM_PAYLOAD: u32 = WM_APP + 1;

fn err(e: windows::core::Error) -> Error {
    e.message()
}

pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

// ------------------------------------------------------------------ messages

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mouse {
    LDown,
    LUp,
    RDown,
    RUp,
    MDown,
    MUp,
    Move,
}

#[derive(Clone, Copy, Debug)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
}

pub enum Msg<'a> {
    /// Control notification or menu item. `code` is the control's notification code (0 for menus).
    Command { id: u16, code: u16 },
    Size,
    Paint(&'a mut Canvas),
    Mouse { kind: Mouse, x: i32, y: i32, mods: Mods },
    Wheel(f32),
    Key(u32),
    Timer(usize),
    Drop(Vec<PathBuf>),
    Payload(Box<dyn Any + Send>),
    Close,
}

pub trait Handler {
    /// Return true if handled; false falls through to DefWindowProcW.
    fn on(&mut self, w: Wnd, msg: Msg) -> bool;
}

thread_local! {
    static HANDLER: RefCell<Option<Box<dyn Handler>>> = const { RefCell::new(None) };
    static DEFERRED: RefCell<VecDeque<(Wnd, Box<dyn Any + Send>)>> = const { RefCell::new(VecDeque::new()) };
    static ENTER: RefCell<Option<(HWND, u16)>> = const { RefCell::new(None) };
    static FONT: RefCell<Option<HFONT>> = const { RefCell::new(None) };
}

/// Dispatch to the handler; None if it's busy (re-entrant call) or not installed yet.
fn dispatch(w: Wnd, msg: Msg) -> Option<bool> {
    let r = HANDLER.with(|h| {
        let mut h = h.try_borrow_mut().ok()?;
        Some(h.as_mut()?.on(w, msg))
    });
    if r.is_some() {
        // Deliver payloads that arrived while we were busy.
        while let Some((w, p)) = DEFERRED.with(|d| d.borrow_mut().pop_front()) {
            HANDLER.with(|h| {
                if let Ok(mut h) = h.try_borrow_mut()
                    && let Some(h) = h.as_mut()
                {
                    h.on(w, Msg::Payload(p));
                }
            });
        }
    }
    r
}

fn mods() -> Mods {
    // SAFETY: GetKeyState has no preconditions.
    let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk.0 as i32) } < 0;
    Mods { shift: down(VK_SHIFT), ctrl: down(VK_CONTROL) }
}

extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(h, m, wp, lp)));
    match r {
        Ok(Some(l)) => l,
        Ok(None) => def(h, m, wp, lp),
        Err(_) => {
            log("panic in window procedure");
            def(h, m, wp, lp)
        }
    }
}

fn def(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: forwarding the exact arguments Windows gave us.
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

fn handle(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let w = Wnd(h);
    let xy = || ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
    let mouse = |kind| {
        let (x, y) = xy();
        dispatch(w, Msg::Mouse { kind, x, y, mods: mods() })
    };
    let handled = match m {
        WM_COMMAND => dispatch(w, Msg::Command { id: (wp.0 & 0xffff) as u16, code: ((wp.0 >> 16) & 0xffff) as u16 }),
        WM_SIZE => dispatch(w, Msg::Size),
        WM_PAINT => {
            let mut c = Canvas::begin(h);
            if dispatch(w, Msg::Paint(&mut c)) != Some(true) {
                // Nobody painted: erase with the class brush instead of blitting an empty buffer.
                c.discard();
                c.erase();
            }
            return Some(LRESULT(0));
        }
        WM_LBUTTONDOWN => mouse(Mouse::LDown),
        WM_LBUTTONUP => mouse(Mouse::LUp),
        WM_RBUTTONDOWN => mouse(Mouse::RDown),
        WM_RBUTTONUP => mouse(Mouse::RUp),
        WM_MBUTTONDOWN => mouse(Mouse::MDown),
        WM_MBUTTONUP => mouse(Mouse::MUp),
        WM_MOUSEMOVE => mouse(Mouse::Move),
        WM_MOUSEWHEEL => dispatch(w, Msg::Wheel(((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0)),
        WM_KEYDOWN => dispatch(w, Msg::Key(wp.0 as u32)),
        WM_TIMER => dispatch(w, Msg::Timer(wp.0)),
        WM_CLOSE => dispatch(w, Msg::Close),
        WM_DROPFILES => {
            let files = dropped_files(HDROP(lp.0 as _));
            dispatch(w, Msg::Drop(files))
        }
        WM_PAYLOAD => {
            // SAFETY: LPARAM was produced by Box::into_raw in Poster::post and is consumed exactly once here.
            let p = unsafe { Box::from_raw(lp.0 as *mut Box<dyn Any + Send>) };
            let busy = HANDLER.with(|c| c.try_borrow_mut().is_err());
            if busy {
                DEFERRED.with(|d| d.borrow_mut().push_back((w, *p)));
                return Some(LRESULT(0));
            }
            dispatch(w, Msg::Payload(*p))
        }
        _ => None,
    };
    match handled {
        Some(true) => Some(LRESULT(0)),
        _ => None,
    }
}

fn dropped_files(drop: HDROP) -> Vec<PathBuf> {
    // SAFETY: HDROP comes from WM_DROPFILES; index u32::MAX queries the count.
    let n = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut out = vec![];
    for i in 0..n {
        let mut buf = [0u16; 1024];
        // SAFETY: buf is a valid writable slice for the duration of the call.
        let len = unsafe { DragQueryFileW(drop, i, Some(&mut buf)) } as usize;
        out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
    }
    // SAFETY: releases the HDROP once, after its last use.
    unsafe { DragFinish(drop) };
    out
}

pub fn log(s: &str) {
    let h = HSTRING::from(format!("zaxis: {s}\n"));
    // SAFETY: h outlives the call.
    unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(&h) };
}

// ------------------------------------------------------------------ app lifetime

/// Initialises COM/DPI, registers classes, lets `build` create windows, then runs the message loop.
pub fn run(build: impl FnOnce() -> Result<Box<dyn Handler>, Error>) -> Result<(), Error> {
    // SAFETY: called once on the UI thread before any window exists.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(err)?;
    }
    register(w!("ZAxisWnd"), true)?;
    register(w!("ZAxisView"), false)?;
    let handler = build()?;
    HANDLER.with(|h| *h.borrow_mut() = Some(handler));

    let mut msg = MSG::default();
    // SAFETY: standard message loop on the thread that owns the windows.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        let enter = ENTER.with(|e| *e.borrow());
        if let Some((target, id)) = enter
            && msg.hwnd == target
            && msg.message == WM_KEYDOWN
            && msg.wParam.0 == VK_RETURN.0 as usize
            && !mods().shift
        {
            // SAFETY: GetParent on a live child window.
            let parent = unsafe { GetParent(target) }.unwrap_or_default();
            dispatch(Wnd(parent), Msg::Command { id, code: 0 });
            continue;
        }
        // SAFETY: msg was filled by GetMessageW.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    HANDLER.with(|h| h.borrow_mut().take());
    // SAFETY: balances CoInitializeEx above on the same thread.
    unsafe { CoUninitialize() };
    Ok(())
}

pub fn quit() {
    // SAFETY: no preconditions.
    unsafe { PostQuitMessage(0) };
}

fn instance() -> HINSTANCE {
    // SAFETY: None = handle of the current exe; never fails for it.
    unsafe { GetModuleHandleW(None) }.map(Into::into).unwrap_or_default()
}

fn register(name: PCWSTR, background: bool) -> Result<(), Error> {
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance(),
        // SAFETY: loading a system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        hbrBackground: if background { HBRUSH((COLOR_BTNFACE.0 + 1) as isize as _) } else { HBRUSH::default() },
        lpszClassName: name,
        ..Default::default()
    };
    // SAFETY: wc and the static class name are valid for the call.
    if unsafe { RegisterClassExW(&wc) } == 0 {
        return Err(err(windows::core::Error::from_thread()));
    }
    Ok(())
}

/// Every Enter (without Shift) in `input` is delivered as `Command { id }` to its parent.
pub fn enter_sends(input: Wnd, id: u16) {
    ENTER.with(|e| *e.borrow_mut() = Some((input.0, id)));
}

// ------------------------------------------------------------------ windows & controls

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Wnd(HWND);

pub enum Ctl {
    Button,
    Static,
    /// Multiline edit.
    Edit { readonly: bool },
    Line,
    Combo,
    List { multi: bool },
    View,
}

/// Sends messages to a window from any thread.
#[derive(Clone, Copy)]
pub struct Poster(isize);

impl Poster {
    pub fn post(&self, p: Box<dyn Any + Send>) {
        let raw = Box::into_raw(Box::new(p));
        // SAFETY: wndproc reclaims `raw` with Box::from_raw when WM_PAYLOAD is dispatched.
        let ok = unsafe { PostMessageW(Some(HWND(self.0 as _)), WM_PAYLOAD, WPARAM(0), LPARAM(raw as isize)) };
        if ok.is_err() {
            // SAFETY: posting failed, so nobody else owns `raw`.
            drop(unsafe { Box::from_raw(raw) });
        }
    }
}

// SAFETY: Poster only stores the window handle as an integer and uses it with PostMessageW,
// which is documented as callable from any thread.
unsafe impl Send for Poster {}

impl Wnd {
    pub fn top(title: &str, w: i32, h: i32, view: bool) -> Result<Wnd, Error> {
        let class = if view { w!("ZAxisView") } else { w!("ZAxisWnd") };
        // SAFETY: class is registered, title HSTRING lives for the call.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                &HSTRING::from(title),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                None,
                None,
                Some(instance()),
                None,
            )
        }
        .map_err(err)?;
        Ok(Wnd(hwnd))
    }

    pub fn child(&self, kind: Ctl, id: u16, text: &str) -> Result<Wnd, Error> {
        let base = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        let (class, style, ex) = match kind {
            Ctl::Button => (w!("BUTTON"), base | WINDOW_STYLE(BS_PUSHBUTTON as u32), WINDOW_EX_STYLE::default()),
            Ctl::Static => (w!("STATIC"), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x000C | 0x0200) /* SS_LEFTNOWORDWRAP | SS_CENTERIMAGE */, WINDOW_EX_STYLE::default()),
            Ctl::Edit { readonly } => {
                let mut s = base | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32);
                if readonly {
                    s |= WINDOW_STYLE(ES_READONLY as u32);
                }
                (w!("EDIT"), s, WS_EX_CLIENTEDGE)
            }
            Ctl::Line => (w!("EDIT"), base | WINDOW_STYLE(ES_AUTOHSCROLL as u32), WS_EX_CLIENTEDGE),
            Ctl::Combo => (w!("COMBOBOX"), base | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32), WINDOW_EX_STYLE::default()),
            Ctl::List { multi } => {
                let mut s = base | WS_VSCROLL | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32);
                if multi {
                    s |= WINDOW_STYLE(LBS_MULTIPLESEL as u32);
                }
                (w!("LISTBOX"), s, WS_EX_CLIENTEDGE)
            }
            Ctl::View => (w!("ZAxisView"), WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS, WINDOW_EX_STYLE::default()),
        };
        // SAFETY: parent is a live window; the control id travels in the HMENU slot as Win32 requires.
        let hwnd = unsafe {
            CreateWindowExW(ex, class, &HSTRING::from(text), style, 0, 0, 10, 10, Some(self.0), Some(HMENU(id as usize as _)), Some(instance()), None)
        }
        .map_err(err)?;
        let c = Wnd(hwnd);
        c.send(WM_SETFONT, font(self.dpi()).0 as usize, 1);
        Ok(c)
    }

    fn send(&self, m: u32, wp: usize, lp: isize) -> isize {
        // SAFETY: messages sent by this module only use integer / caller-owned-buffer arguments.
        unsafe { SendMessageW(self.0, m, Some(WPARAM(wp)), Some(LPARAM(lp))) }.0
    }

    pub fn poster(&self) -> Poster {
        Poster(self.0.0 as isize)
    }

    pub fn raw(&self) -> isize {
        self.0.0 as isize
    }

    pub fn dpi(&self) -> f32 {
        // SAFETY: valid window handle.
        let d = unsafe { GetDpiForWindow(self.0) };
        if d == 0 { 1.0 } else { d as f32 / 96.0 }
    }

    pub fn set_pos(&self, x: i32, y: i32, w: i32, h: i32) {
        // SAFETY: valid window; no z-order change.
        let _ = unsafe { SetWindowPos(self.0, None, x, y, w.max(0), h.max(0), SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    pub fn client_size(&self) -> (i32, i32) {
        let mut r = RECT::default();
        // SAFETY: r is a valid out-pointer.
        let _ = unsafe { GetClientRect(self.0, &mut r) };
        (r.right - r.left, r.bottom - r.top)
    }

    pub fn show(&self, on: bool) {
        // SAFETY: valid window.
        let _ = unsafe { ShowWindow(self.0, if on { SW_SHOW } else { SW_HIDE }) };
    }

    pub fn visible(&self) -> bool {
        // SAFETY: valid window.
        unsafe { IsWindowVisible(self.0) }.as_bool()
    }

    pub fn enable(&self, on: bool) {
        // SAFETY: valid window.
        let _ = unsafe { EnableWindow(self.0, on) };
    }

    pub fn focus(&self) {
        // SAFETY: valid window on this thread.
        let _ = unsafe { SetFocus(Some(self.0)) };
    }

    pub fn invalidate(&self) {
        // SAFETY: valid window; None = whole client area.
        let _ = unsafe { InvalidateRect(Some(self.0), None, false) };
    }

    pub fn capture(&self, on: bool) {
        // SAFETY: valid window on this thread.
        unsafe {
            if on {
                SetCapture(self.0);
            } else {
                let _ = ReleaseCapture();
            }
        }
    }

    pub fn set_timer(&self, id: usize, ms: u32) {
        // SAFETY: valid window; no callback, so WM_TIMER is posted.
        unsafe { SetTimer(Some(self.0), id, ms, None) };
    }

    pub fn accept_files(&self) {
        // SAFETY: valid window.
        unsafe { DragAcceptFiles(self.0, true) };
    }

    pub fn set_text(&self, s: &str) {
        // SAFETY: HSTRING lives for the call.
        let _ = unsafe { SetWindowTextW(self.0, &HSTRING::from(s)) };
    }

    pub fn text(&self) -> String {
        // SAFETY: valid window.
        let n = unsafe { GetWindowTextLengthW(self.0) } as usize;
        let mut buf = vec![0u16; n + 1];
        // SAFETY: buf has room for n chars + NUL.
        let len = unsafe { GetWindowTextW(self.0, &mut buf) } as usize;
        String::from_utf16_lossy(&buf[..len])
    }

    /// Appends to an edit control and scrolls to the end. Converts \n to the \r\n EDIT needs.
    pub fn append(&self, s: &str) {
        let h = HSTRING::from(s.replace("\r\n", "\n").replace('\n', "\r\n"));
        let end = self.send(WM_GETTEXTLENGTH, 0, 0) as usize;
        self.send(EM_SETSEL, end, end as isize);
        // SAFETY: h is NUL-terminated and outlives the synchronous SendMessage.
        self.send(EM_REPLACESEL, 0, h.as_ptr() as isize);
        self.send(EM_SCROLLCARET, 0, 0);
    }

    pub fn combo_add(&self, s: &str) {
        let h = HSTRING::from(s);
        self.send(CB_ADDSTRING, 0, h.as_ptr() as isize);
    }

    pub fn combo_sel(&self) -> Option<usize> {
        usize::try_from(self.send(CB_GETCURSEL, 0, 0)).ok()
    }

    pub fn combo_set_sel(&self, i: usize) {
        self.send(CB_SETCURSEL, i, 0);
    }

    pub fn list_clear(&self) {
        self.send(LB_RESETCONTENT, 0, 0);
    }

    pub fn list_add(&self, s: &str) {
        let h = HSTRING::from(s);
        self.send(LB_ADDSTRING, 0, h.as_ptr() as isize);
    }

    pub fn list_sel(&self) -> Option<usize> {
        usize::try_from(self.send(LB_GETCURSEL, 0, 0)).ok()
    }

    pub fn list_set_sel(&self, i: usize) {
        self.send(LB_SETCURSEL, i, 0);
    }

    pub fn list_multi_sel(&self) -> Vec<usize> {
        let n = self.send(LB_GETSELCOUNT, 0, 0).max(0) as usize;
        let mut v = vec![0i32; n];
        let got = self.send(LB_GETSELITEMS, n, v.as_mut_ptr() as isize).max(0) as usize;
        v.truncate(got);
        v.into_iter().map(|i| i as usize).collect()
    }

    pub fn list_set_multi(&self, i: usize, on: bool) {
        self.send(LB_SETSEL, on as usize, i as isize);
    }

    /// Menu bar from `(title, [(id, label)])`; id 0 = separator.
    pub fn set_menu(&self, menus: &[(&str, &[(u16, &str)])]) -> Result<(), Error> {
        // SAFETY: menus are handed to the window, which destroys them with itself.
        unsafe {
            let bar = CreateMenu().map_err(err)?;
            for (title, items) in menus {
                let pop = CreatePopupMenu().map_err(err)?;
                for &(id, label) in *items {
                    if id == 0 {
                        AppendMenuW(pop, MF_SEPARATOR, 0, None).map_err(err)?;
                    } else {
                        AppendMenuW(pop, MF_STRING, id as usize, &HSTRING::from(label)).map_err(err)?;
                    }
                }
                AppendMenuW(bar, MF_POPUP, pop.0 as usize, &HSTRING::from(*title)).map_err(err)?;
            }
            SetMenu(self.0, Some(bar)).map_err(err)?;
        }
        Ok(())
    }

    pub fn check_menu(&self, id: u16, on: bool) {
        // SAFETY: valid window; a missing menu just makes the call a no-op.
        unsafe {
            let m = GetMenu(self.0);
            CheckMenuItem(m, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
        }
    }

    /// Modal multi-select file picker filtered to images. Empty on cancel.
    pub fn open_images(&self) -> Vec<PathBuf> {
        self.open_images_inner().unwrap_or_default()
    }

    fn open_images_inner(&self) -> windows::core::Result<Vec<PathBuf>> {
        // SAFETY: COM was initialised in `run`; all strings outlive the dialog calls.
        unsafe {
            let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dlg.SetOptions(dlg.GetOptions()? | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST)?;
            let filter = [COMDLG_FILTERSPEC { pszName: w!("Images"), pszSpec: w!("*.png;*.jpg;*.jpeg;*.webp;*.bmp") }];
            dlg.SetFileTypes(&filter)?;
            dlg.Show(Some(self.0))?;
            let items = dlg.GetResults()?;
            let mut out = vec![];
            for i in 0..items.GetCount()? {
                let p = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
                out.push(PathBuf::from(p.to_string().unwrap_or_default()));
                CoTaskMemFree(Some(p.0 as _));
            }
            Ok(out)
        }
    }
}

/// Shared UI font for this DPI scale. Created once and kept for the process lifetime.
fn font(scale: f32) -> HFONT {
    FONT.with(|f| {
        *f.borrow_mut().get_or_insert_with(|| {
            // SAFETY: creating a GDI font with a static face name.
            unsafe {
                CreateFontW(
                    -(15.0 * scale) as i32,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                )
            }
        })
    })
}

// ------------------------------------------------------------------ GDI canvas (double-buffered)

pub struct Canvas {
    hwnd: HWND,
    ps: PAINTSTRUCT,
    screen: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    blit: bool,
    pub w: i32,
    pub h: i32,
}

impl Canvas {
    fn begin(hwnd: HWND) -> Canvas {
        let mut ps = PAINTSTRUCT::default();
        let mut r = RECT::default();
        // SAFETY: called from WM_PAINT for this window; Drop calls EndPaint and frees the GDI objects.
        unsafe {
            let screen = BeginPaint(hwnd, &mut ps);
            let _ = GetClientRect(hwnd, &mut r);
            let (w, h) = (r.right.max(1), r.bottom.max(1));
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp.into());
            Canvas { hwnd, ps, screen, mem, bmp, old, blit: true, w, h }
        }
    }

    /// Validate the window without drawing (for D3D-rendered views).
    pub fn discard(&mut self) {
        self.blit = false;
    }

    fn erase(&mut self) {
        if self.ps.fErase.as_bool() {
            // SAFETY: screen DC from BeginPaint; class brush belongs to the window class.
            unsafe {
                let brush = HBRUSH(GetClassLongPtrW(self.hwnd, GCLP_HBRBACKGROUND) as _);
                if !brush.is_invalid() {
                    FillRect(self.screen, &self.ps.rcPaint, brush);
                }
            }
        }
    }

    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        // SAFETY: mem is a live DC; the brush is deleted right after use.
        unsafe {
            let b = CreateSolidBrush(COLORREF(color));
            FillRect(self.mem, &RECT { left: x, top: y, right: x + w, bottom: y + h }, b);
            let _ = DeleteObject(b.into());
        }
    }

    pub fn image(&mut self, img: &crate::glb::Image, x: i32, y: i32, w: i32, h: i32) {
        let (rgba, iw, ih) = (&img.rgba, img.width, img.height);
        if rgba.len() < (iw * ih * 4) as usize {
            return;
        }
        let bgra: Vec<u8> = rgba.chunks(4).flat_map(|c| [c[2], c[1], c[0], 255]).collect();
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: iw as i32,
                biHeight: -(ih as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: bgra holds iw*ih 32-bit pixels described by bi and outlives the call.
        unsafe {
            SetStretchBltMode(self.mem, HALFTONE);
            StretchDIBits(self.mem, x, y, w, h, 0, 0, iw as i32, ih as i32, Some(bgra.as_ptr() as _), &bi, DIB_RGB_COLORS, SRCCOPY);
        }
    }

    pub fn lines(&mut self, segs: &[(f32, f32, f32, f32)], color: u32) {
        // SAFETY: mem is live; the pen is selected out and deleted before returning.
        unsafe {
            let pen = CreatePen(PS_SOLID, 1, COLORREF(color));
            let old = SelectObject(self.mem, pen.into());
            for &(x0, y0, x1, y1) in segs {
                let _ = MoveToEx(self.mem, x0 as i32, y0 as i32, None);
                let _ = LineTo(self.mem, x1 as i32, y1 as i32);
            }
            SelectObject(self.mem, old);
            let _ = DeleteObject(pen.into());
        }
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, color: u32) {
        let t: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: t outlives the call; font handle is process-lifetime.
        unsafe {
            let old = SelectObject(self.mem, font(1.0).into());
            SetBkMode(self.mem, TRANSPARENT);
            SetTextColor(self.mem, COLORREF(color));
            let _ = TextOutW(self.mem, x, y, &t);
            SelectObject(self.mem, old);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: blits the back buffer, restores/frees the objects created in `begin`, ends painting once.
        unsafe {
            if self.blit {
                let _ = BitBlt(self.screen, 0, 0, self.w, self.h, Some(self.mem), 0, 0, SRCCOPY);
            }
            SelectObject(self.mem, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.mem);
            let _ = EndPaint(self.hwnd, &self.ps);
        }
    }
}
