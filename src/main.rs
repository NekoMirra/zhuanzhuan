#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

// ==================== Win32 FFI ====================
#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;
    pub type HWND = isize;
    pub type HDC = isize;
    pub type HBITMAP = isize;
    pub type HGDIOBJ = isize;

    #[repr(C)] pub struct POINT { pub x: i32, pub y: i32 }
    #[repr(C)] pub struct SIZE { pub cx: i32, pub cy: i32 }
    #[repr(C)] pub struct BLENDFUNCTION { pub op: u8, pub flags: u8, pub alpha: u8, pub format: u8 }
    #[repr(C)]
    pub struct BITMAPINFOHEADER {
        pub size: u32, pub width: i32, pub height: i32,
        pub planes: u16, pub bit_count: u16, pub compression: u32,
        pub size_image: u32, pub x_ppm: i32, pub y_ppm: i32,
        pub clr_used: u32, pub clr_important: u32,
    }
    #[repr(C)] pub struct BITMAPINFO { pub header: BITMAPINFOHEADER, pub colors: [u32; 1] }
    #[repr(C)]
    pub struct WNDCLASSEXW {
        pub size: u32, pub style: u32,
        pub wnd_proc: Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>,
        pub cls_extra: i32, pub wnd_extra: i32, pub instance: isize,
        pub icon: isize, pub cursor: isize, pub background: isize,
        pub menu_name: *const u16, pub class_name: *const u16, pub icon_sm: isize,
    }
    extern "system" {
        pub fn GetSystemMetrics(i: i32) -> i32;
        pub fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
        pub fn CreateWindowExW(ex: u32, cls: *const u16, title: *const u16, style: u32,
            x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: isize, inst: isize, param: *mut c_void) -> HWND;
        pub fn ShowWindow(h: HWND, cmd: i32) -> i32;
        pub fn DefWindowProcW(h: HWND, msg: u32, wp: usize, lp: isize) -> isize;
        pub fn GetModuleHandleW(name: *const u16) -> isize;
        pub fn GetDC(h: HWND) -> HDC;
        pub fn ReleaseDC(h: HWND, dc: HDC) -> i32;
        pub fn CreateCompatibleDC(dc: HDC) -> HDC;
        pub fn DeleteDC(dc: HDC) -> i32;
        pub fn CreateDIBSection(dc: HDC, bmi: *const BITMAPINFO, usage: u32,
            bits: *mut *mut c_void, section: isize, offset: u32) -> HBITMAP;
        pub fn SelectObject(dc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
        pub fn DeleteObject(obj: HGDIOBJ) -> i32;
        pub fn UpdateLayeredWindow(h: HWND, dst_dc: HDC, dst_pt: *const POINT, sz: *const SIZE,
            src_dc: HDC, src_pt: *const POINT, key: u32, blend: *const BLENDFUNCTION, flags: u32) -> i32;
        pub fn FindWindowW(cls: *const u16, title: *const u16) -> HWND;
        pub fn SetForegroundWindow(h: HWND) -> i32;
        pub fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }
}

// ==================== Transparent overlay ====================
const OVR: i32 = 300;
const OVR_HALF: f32 = OVR as f32 / 2.0;

struct Overlay { hwnd: win32::HWND }
unsafe impl Send for Overlay {}

impl Overlay {
    fn new() -> Self {
        unsafe {
            let cls: Vec<u16> = "ZZO\0".encode_utf16().collect();
            let inst = win32::GetModuleHandleW(std::ptr::null());
            let wc = win32::WNDCLASSEXW {
                size: std::mem::size_of::<win32::WNDCLASSEXW>() as u32,
                wnd_proc: Some(win32::DefWindowProcW), instance: inst, class_name: cls.as_ptr(),
                style: 0, cls_extra: 0, wnd_extra: 0, icon: 0, cursor: 0,
                background: 0, menu_name: std::ptr::null(), icon_sm: 0,
            };
            win32::RegisterClassExW(&wc);
            let ex = 0x00080000 | 0x00000020 | 0x00000008 | 0x00000080 | 0x08000000;
            let hwnd = win32::CreateWindowExW(ex, cls.as_ptr(), cls.as_ptr(), 0x80000000u32,
                0, 0, OVR, OVR, 0, 0, inst, std::ptr::null_mut());
            Self { hwnd }
        }
    }
    fn show(&self) { unsafe { win32::ShowWindow(self.hwnd, 5); } }
    fn hide(&self) { unsafe { win32::ShowWindow(self.hwnd, 0); } }
    fn render(&self, x: i32, y: i32, draw: impl FnOnce(&mut [u8], i32, i32)) {
        unsafe {
            let sdc = win32::GetDC(0);
            let mdc = win32::CreateCompatibleDC(sdc);
            let mut bmi: win32::BITMAPINFO = std::mem::zeroed();
            bmi.header.size = std::mem::size_of::<win32::BITMAPINFOHEADER>() as u32;
            bmi.header.width = OVR; bmi.header.height = -OVR;
            bmi.header.planes = 1; bmi.header.bit_count = 32;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = win32::CreateDIBSection(mdc, &bmi, 0, &mut bits, 0, 0);
            let old = win32::SelectObject(mdc, bmp);
            let buf = std::slice::from_raw_parts_mut(bits as *mut u8, (OVR * OVR * 4) as usize);
            buf.fill(0);
            draw(buf, OVR, OVR);
            let pt = win32::POINT { x, y };
            let sz = win32::SIZE { cx: OVR, cy: OVR };
            let src = win32::POINT { x: 0, y: 0 };
            let blend = win32::BLENDFUNCTION { op: 0, flags: 0, alpha: 255, format: 1 };
            win32::UpdateLayeredWindow(self.hwnd, sdc, &pt, &sz, mdc, &src, 0, &blend, 2);
            win32::SelectObject(mdc, old);
            win32::DeleteObject(bmp);
            win32::DeleteDC(mdc);
            win32::ReleaseDC(0, sdc);
        }
    }
}

fn draw_glow(buf: &mut [u8], bw: i32, _bh: i32, cx: f32, cy: f32, radius: f32, r: u8, g: u8, b: u8, a: u8) {
    let r2 = radius * radius;
    let x0 = (cx - radius).max(0.0) as i32;
    let y0 = (cy - radius).max(0.0) as i32;
    let x1 = (cx + radius).min(bw as f32 - 1.0) as i32;
    let y1 = (cy + radius).min(bw as f32 - 1.0) as i32;
    for py in y0..=y1 {
        for px in x0..=x1 {
            let dx = px as f32 - cx; let dy = py as f32 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 < r2 {
                let edge = (1.0 - (d2.sqrt() / radius)).max(0.0);
                let fa = a as f32 / 255.0 * edge;
                let idx = ((py * bw + px) * 4) as usize;
                buf[idx]     = (buf[idx] as f32 + b as f32 * fa).min(255.0) as u8;
                buf[idx + 1] = (buf[idx + 1] as f32 + g as f32 * fa).min(255.0) as u8;
                buf[idx + 2] = (buf[idx + 2] as f32 + r as f32 * fa).min(255.0) as u8;
                buf[idx + 3] = (buf[idx + 3] as f32 + 255.0 * fa).min(255.0) as u8;
            }
        }
    }
}

fn screen_edge_pos(progress: f32) -> (egui::Pos2, egui::Vec2) {
    let sw = unsafe { win32::GetSystemMetrics(0) } as f32;
    let sh = unsafe { win32::GetSystemMetrics(1) } as f32;
    let m = 8.0;
    let (ew, eh) = (sw - 2.0 * m, sh - 2.0 * m);
    let peri = 2.0 * (ew + eh);
    let d = (progress.fract() + 1.0).fract() * peri;
    if d < ew {
        (egui::pos2(m + d, m), egui::vec2(1.0, 0.0))
    } else if d < ew + eh {
        (egui::pos2(sw - m, m + d - ew), egui::vec2(0.0, 1.0))
    } else if d < 2.0 * ew + eh {
        (egui::pos2(sw - m - (d - (ew + eh)), sh - m), egui::vec2(-1.0, 0.0))
    } else {
        (egui::pos2(m, sh - m - (d - (2.0 * ew + eh))), egui::vec2(0.0, -1.0))
    }
}

// ==================== Background timer thread ====================
enum Cmd { Start { interval_s: u64, lap_dur: f32, laps: f32 }, Stop }

#[derive(Clone)]
enum Status { Idle, Waiting { remaining: u64 }, Running { elapsed: f32, total: f32 } }

// TrayIcon is !Send but set_tooltip uses Shell_NotifyIconW which is thread-safe
struct SendTray(tray_icon::TrayIcon);
unsafe impl Send for SendTray {}
unsafe impl Sync for SendTray {}

fn timer_thread(cmd_rx: mpsc::Receiver<Cmd>, status: Arc<Mutex<Status>>, tray: Arc<SendTray>) {
    let mut params: Option<(u64, f32, f32)> = None;

    loop {
        match cmd_rx.recv() {
            Ok(Cmd::Start { interval_s, lap_dur, laps }) => {
                params = Some((interval_s, lap_dur, laps));
            }
            Ok(Cmd::Stop) => { *status.lock().unwrap() = Status::Idle; continue; }
            Err(_) => return,
        }

        while let Some((interval_s, lap_dur, laps)) = params {
            // === WAIT PHASE ===
            let deadline = Instant::now() + Duration::from_secs(interval_s);
            let mut stopped = false;
            while Instant::now() < deadline {
                if let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        Cmd::Stop => { stopped = true; break; }
                        Cmd::Start { interval_s: i, lap_dur: d, laps: l } => {
                            params = Some((i, d, l));
                        }
                    }
                }
                let rem = (deadline - Instant::now()).as_secs();
                *status.lock().unwrap() = Status::Waiting { remaining: rem };
                // Update tray tooltip directly from thread
                let _ = tray.0.set_tooltip(Some(&format!("转转 - 距下次放松 {:02}:{:02}", rem / 60, rem % 60)));
                std::thread::sleep(Duration::from_secs(1));
            }
            if stopped { *status.lock().unwrap() = Status::Idle; params = None;
                let _ = tray.0.set_tooltip(Some("转转 - 未开始"));
                break;
            }

            // === ANIMATION PHASE ===
            let overlay = Overlay::new();
            overlay.show();
            let start = Instant::now();
            let total = lap_dur * laps;
            let mut stopped = false;

            loop {
                let elapsed = (Instant::now() - start).as_secs_f32();
                if elapsed > total { break; }

                if let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        Cmd::Stop => { stopped = true; break; }
                        Cmd::Start { interval_s: i, lap_dur: d, laps: l } => {
                            params = Some((i, d, l));
                        }
                    }
                }
                *status.lock().unwrap() = Status::Running { elapsed, total };
                let _ = tray.0.set_tooltip(Some("转转 - 放松中..."));

                // Render
                let progress = (elapsed % lap_dur) / lap_dur;
                let (head, travel) = screen_edge_pos(progress);
                let trail = egui::vec2(-travel.x, -travel.y);
                let ox = (head.x - OVR_HALF) as i32;
                let oy = (head.y - OVR_HALF) as i32;

                // Force overlay above taskbar every frame
                unsafe { win32::SetWindowPos(overlay.hwnd, -1, ox, oy, OVR, OVR, 0x0010); } // HWND_TOPMOST=-1, SWP_NOACTIVATE=0x0010

                overlay.render(ox, oy, |buf, bw, bh| {
                    let (cx, cy) = (OVR_HALF, OVR_HALF);
                    for i in (0..40).rev() {
                        let t = i as f32 / 40.0;
                        let fade = (1.0 - t).powi(2);
                        draw_glow(buf, bw, bh,
                            cx + trail.x * t * 200.0, cy + trail.y * t * 200.0,
                            3.0 + 14.0 * fade, 0, 255, 255, (220.0 * fade) as u8);
                    }
                    draw_glow(buf, bw, bh, cx, cy, 20.0, 0, 200, 255, 80);
                    draw_glow(buf, bw, bh, cx, cy, 12.0, 100, 255, 255, 180);
                    draw_glow(buf, bw, bh, cx, cy, 5.0, 255, 255, 255, 255);
                });

                std::thread::sleep(Duration::from_millis(16)); // ~60fps
            }
            overlay.hide();
            if stopped { *status.lock().unwrap() = Status::Idle; params = None;
                let _ = tray.0.set_tooltip(Some("转转 - 未开始"));
                break;
            }
            // Loop back to wait phase with same params
        }
    }
}

// ==================== Config persistence ====================
fn config_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.set_file_name("zhuanzhuan.conf");
    p
}
fn load_config() -> (String, String, String) {
    let (mut a, mut b, mut c) = ("20".into(), "8".into(), "3".into());
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "interval" => a = v.trim().into(),
                    "duration" => b = v.trim().into(),
                    "laps" => c = v.trim().into(),
                    _ => {}
                }
            }
        }
    }
    (a, b, c)
}
fn save_config(a: &str, b: &str, c: &str) {
    let _ = std::fs::write(config_path(), format!("interval={a}\nduration={b}\nlaps={c}\n"));
}

// ==================== App ====================
fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

unsafe fn show_main_window(hwnd: isize) {
    if hwnd != 0 {
        win32::ShowWindow(hwnd, 5);  // SW_SHOW
        win32::ShowWindow(hwnd, 9);  // SW_RESTORE
        win32::SetForegroundWindow(hwnd);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 320.0])
            .with_title("转转"),
        ..Default::default()
    };
    eframe::run_native("转转", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct App {
    interval_min: String, lap_sec: String, laps: String,
    active: bool,
    cmd_tx: mpsc::Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    hwnd: Arc<AtomicIsize>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Visuals
        let mut v = egui::Visuals::dark();
        v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 30, 36);
        v.widgets.inactive.bg_fill = egui::Color32::from_rgb(45, 45, 55);
        v.widgets.hovered.bg_fill = egui::Color32::from_rgb(55, 55, 70);
        v.widgets.active.bg_fill = egui::Color32::from_rgb(0, 180, 200);
        v.selection.bg_fill = egui::Color32::from_rgb(0, 160, 180);
        cc.egui_ctx.set_visuals(v);

        // Chinese font
        let mut fonts = egui::FontDefinitions::default();
        if let Ok(data) = std::fs::read("C:\\Windows\\Fonts\\msyh.ttc") {
            fonts.font_data.insert("msyh".into(), std::sync::Arc::new(egui::FontData::from_owned(data)));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "msyh".into());
            cc.egui_ctx.set_fonts(fonts);
        }

        let (interval_min, lap_sec, laps) = load_config();

        // Background timer thread (started later after tray is built)
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::Idle));

        // Auto-launch
        let exe = std::env::current_exe().unwrap().to_str().unwrap().to_string();
        let exe2 = exe.clone();
        let al = auto_launch::AutoLaunchBuilder::new()
            .set_app_name("Zhuanzhuan").set_app_path(&exe).build().unwrap();
        let enabled = al.is_enabled().unwrap_or(false);

        // Tray
        let menu = tray_icon::menu::Menu::new();
        let _ = menu.append_items(&[
            &tray_icon::menu::MenuItem::with_id("show", "打开设置", true, None),
            &tray_icon::menu::CheckMenuItem::with_id("autostart", "开机自启", true, enabled, None),
            &tray_icon::menu::PredefinedMenuItem::separator(),
            &tray_icon::menu::MenuItem::with_id("exit", "退出", true, None),
        ]);
        let icon_data = include_bytes!("../icon.png");
        let img = image::load_from_memory(icon_data).unwrap().into_rgba8();
        let (w, h) = img.dimensions();
        let icon = tray_icon::Icon::from_rgba(img.into_raw(), w, h).unwrap();

        let hwnd = Arc::new(AtomicIsize::new(0));
        let hwnd_menu = hwnd.clone();
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            let h = hwnd_menu.load(Ordering::Relaxed);
            match event.id.0.as_str() {
                "show" => unsafe { show_main_window(h); },
                "exit" => std::process::exit(0),
                "autostart" => {
                    let a = auto_launch::AutoLaunchBuilder::new()
                        .set_app_name("Zhuanzhuan").set_app_path(&exe2).build().unwrap();
                    if a.is_enabled().unwrap_or(false) { let _ = a.disable(); }
                    else { let _ = a.enable(); }
                }
                _ => {}
            }
        }));
        let hwnd_click = hwnd.clone();
        tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
            if let tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up, ..
            } = event {
                unsafe { show_main_window(hwnd_click.load(Ordering::Relaxed)); }
            }
        }));

        let tray = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu)).with_tooltip("转转").with_icon(icon).build().unwrap();

        // Start background thread with shared tray for tooltip updates
        let tray_arc = Arc::new(SendTray(tray));
        let status2 = status.clone();
        std::thread::spawn({
            let tray_arc2 = tray_arc.clone();
            move || timer_thread(cmd_rx, status2, tray_arc2)
        });

        Self {
            interval_min, lap_sec, laps, active: false,
            cmd_tx, status, hwnd,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Store HWND on first frame
        if self.hwnd.load(Ordering::Relaxed) == 0 {
            let title = wide("转转");
            unsafe {
                let h = win32::FindWindowW(std::ptr::null(), title.as_ptr());
                if h != 0 { self.hwnd.store(h, Ordering::Relaxed); }
            }
        }

        // Close → truly hide via Win32
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let h = self.hwnd.load(Ordering::Relaxed);
            if h != 0 { unsafe { win32::ShowWindow(h, 0); } } // SW_HIDE
        }

        let accent = egui::Color32::from_rgb(0, 200, 220);

        // Read thread status (tooltip is updated by background thread directly)
        let st = self.status.lock().unwrap().clone();

        if !self.active {
            // === CONFIG PAGE ===
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(16.0);
                    ui.heading(egui::RichText::new("旧首级，当然要转转").size(28.0).color(accent));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("跟随光流转动，放松脖子和眼睛").size(13.0).color(egui::Color32::GRAY));
                    ui.add_space(16.0);
                });
                ui.separator();
                ui.add_space(10.0);

                let prev = (self.interval_min.clone(), self.lap_sec.clone(), self.laps.clone());
                egui::Grid::new("s").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("⏱  触发间隔");
                    ui.horizontal(|ui| { ui.add(egui::TextEdit::singleline(&mut self.interval_min).desired_width(60.0)); ui.label("分钟"); });
                    ui.end_row();
                    ui.label("🌀  单圈耗时");
                    ui.horizontal(|ui| { ui.add(egui::TextEdit::singleline(&mut self.lap_sec).desired_width(60.0)); ui.label("秒"); });
                    ui.end_row();
                    ui.label("🔢  旋转圈数");
                    ui.horizontal(|ui| { ui.add(egui::TextEdit::singleline(&mut self.laps).desired_width(60.0)); ui.label("圈"); });
                    ui.end_row();
                });
                if (self.interval_min.clone(), self.lap_sec.clone(), self.laps.clone()) != prev {
                    save_config(&self.interval_min, &self.lap_sec, &self.laps);
                }

                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    let btn = egui::Button::new(egui::RichText::new("▶  开始专注").size(16.0)).min_size(egui::vec2(180.0, 36.0));
                    if ui.add(btn).clicked() {
                        if let (Ok(i), Ok(d), Ok(l)) = (
                            self.interval_min.parse::<u64>(),
                            self.lap_sec.parse::<f32>(),
                            self.laps.parse::<f32>(),
                        ) {
                            let _ = self.cmd_tx.send(Cmd::Start { interval_s: i * 60, lap_dur: d, laps: l });
                            self.active = true;
                        }
                    }
                    ui.add_space(6.0);
                    if ui.small_button("立即测试光流").clicked() {
                        if let (Ok(d), Ok(l)) = (self.lap_sec.parse::<f32>(), self.laps.parse::<f32>()) {
                            let _ = self.cmd_tx.send(Cmd::Start { interval_s: 0, lap_dur: d, laps: l });
                            self.active = true;
                        }
                    }
                });
            });
        } else {
            // === ACTIVE PAGE (reading from thread status) ===
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    match st {
                        Status::Waiting { remaining } => {
                            ui.add_space(40.0);
                            ui.label(egui::RichText::new("☕ 专注中").size(22.0).color(accent));
                            ui.add_space(16.0);
                            ui.label(egui::RichText::new(format!("{:02}:{:02}", remaining / 60, remaining % 60))
                                .size(48.0).color(egui::Color32::WHITE).strong());
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("后开始放松").size(13.0).color(egui::Color32::GRAY));
                        }
                        Status::Running { elapsed, total } => {
                            ui.add_space(24.0);
                            ui.label(egui::RichText::new("放松中").size(22.0).color(accent));
                            ui.add_space(8.0);
                            ui.label("请让头部跟随屏幕边缘的光流转动");
                            ui.add_space(12.0);
                            let dur = self.lap_sec.parse::<f32>().unwrap_or(8.0);
                            let cur = (elapsed / dur) as i32 + 1;
                            let nlaps = self.laps.parse::<i32>().unwrap_or(3);
                            ui.add(egui::ProgressBar::new(elapsed / total)
                                .text(format!("第 {}/{} 圈", cur, nlaps)).fill(accent));
                        }
                        Status::Idle => {
                            // Thread finished or something unexpected
                            ui.add_space(40.0);
                            ui.label("已完成");
                        }
                    }
                    ui.add_space(24.0);
                    if ui.button("停止并返回设置").clicked() {
                        let _ = self.cmd_tx.send(Cmd::Stop);
                        self.active = false;
                    }
                });
            });
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}
