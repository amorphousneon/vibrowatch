use eframe::egui::{
    self, epaint::PathShape, text::CCursor, text::CCursorRange, Align2, Color32, FontData,
    FontDefinitions, FontFamily, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2,
};
use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ============================================================
//  CONFIGURATION & DEFAULT VIEW BOUNDS
// ============================================================
const BAUD_RATE: u32 = 115_200;
const MAX_POINTS: usize = 200;

// Default Y-Axis centered at 512 (LM358 DC Bias)
const CENTER_Y: f32 = 512.0;
const Y_SPAN: f32 = 300.0;
const DEFAULT_Y_MIN: f32 = CENTER_Y - Y_SPAN; // 212.0
const DEFAULT_Y_MAX: f32 = CENTER_Y + Y_SPAN; // 812.0
const DEFAULT_X_MIN: f32 = 0.0;
const DEFAULT_X_MAX: f32 = (MAX_POINTS - 1) as f32;

#[derive(PartialEq, Clone, Copy)]
enum ActiveTool {
    None,
    Pan,
    Zoom,
}

#[derive(Clone, Copy)]
struct ViewBounds {
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
}

impl Default for ViewBounds {
    fn default() -> Self {
        Self {
            x_min: DEFAULT_X_MIN,
            x_max: DEFAULT_X_MAX,
            y_min: DEFAULT_Y_MIN,
            y_max: DEFAULT_Y_MAX,
        }
    }
}

// Shared state between the Serial thread and GUI thread
struct TelemetryState {
    target_port: Option<String>,
    wave_data: VecDeque<f32>,
    freq: f32,
    status: String,
    baseline: f32,
    cal_count: u32,
    connected: bool,
    port_error: String,
}

impl Default for TelemetryState {
    fn default() -> Self {
        let mut wave = VecDeque::with_capacity(MAX_POINTS);
        for _ in 0..MAX_POINTS {
            wave.push_back(CENTER_Y);
        }
        Self {
            target_port: None,
            wave_data: wave,
            freq: 0.0,
            status: "IDLE".to_string(),
            baseline: 0.0,
            cal_count: 0,
            connected: false,
            port_error: "No port selected".to_string(),
        }
    }
}

// ============================================================
//  BACKGROUND SERIAL THREAD (DYNAMIC PORT SUPPORT)
// ============================================================
fn spawn_serial_worker(state: Arc<Mutex<TelemetryState>>, ctx: egui::Context) {
    thread::spawn(move || {
        let mut rx_buffer = String::with_capacity(2048);
        let mut read_buf = [0u8; 512];

        loop {
            let current_port = {
                let st = state.lock().unwrap();
                st.target_port.clone()
            };

            let Some(port_name) = current_port else {
                thread::sleep(Duration::from_millis(100));
                continue;
            };

            match serialport::new(&port_name, BAUD_RATE)
            .timeout(Duration::from_millis(20))
            .open()
            {
                Ok(mut port) => {
                    {
                        let mut st = state.lock().unwrap();
                        st.connected = true;
                        st.port_error.clear();
                    }
                    ctx.request_repaint();

                    loop {
                        {
                            let st = state.lock().unwrap();
                            if st.target_port.as_deref() != Some(port_name.as_str()) {
                                break;
                            }
                        }

                        match port.read(&mut read_buf) {
                            Ok(n) if n > 0 => {
                                let chunk = String::from_utf8_lossy(&read_buf[..n]);
                                rx_buffer.push_str(&chunk);

                                if let Some(last_newline) = rx_buffer.rfind('\n') {
                                    let complete_block = rx_buffer[..=last_newline].to_string();
                                    rx_buffer = rx_buffer[last_newline + 1..].to_string();

                                    let mut st = state.lock().unwrap();
                                    let mut got_tel = false;
                                    for raw_line in complete_block.lines() {
                                        let line = raw_line.trim();
                                        if line.is_empty() {
                                            continue;
                                        }
                                        if line.starts_with("$TEL") {
                                            parse_tel_packet(line, &mut st);
                                            got_tel = true;
                                        } else if let Ok(val) = line.parse::<f32>() {
                                            if st.wave_data.len() >= MAX_POINTS {
                                                st.wave_data.pop_front();
                                            }
                                            st.wave_data.push_back(val);
                                        }
                                    }
                                    drop(st);
                                    if got_tel {
                                        ctx.request_repaint();
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                            Err(e) => {
                                let mut st = state.lock().unwrap();
                                st.connected = false;
                                st.port_error = format!("Disconnected ({}): {}", port_name, e);
                                drop(st);
                                ctx.request_repaint();
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    {
                        let mut st = state.lock().unwrap();
                        st.connected = false;
                        st.port_error = format!("Waiting for {} ({})", port_name, e);
                    }
                    ctx.request_repaint();
                    thread::sleep(Duration::from_millis(800));
                }
            }
        }
    });
}

fn parse_tel_packet(packet: &str, st: &mut TelemetryState) {
    let parts: Vec<&str> = packet.split(',').collect();
    if parts.len() != 5 {
        return;
    }
    if let (Ok(freq), Ok(base), Ok(cal)) = (
        parts[1].trim().parse::<f32>(),
                                            parts[3].trim().parse::<f32>(),
                                            parts[4].trim().parse::<u32>(),
    ) {
        st.freq = freq;
        st.status = parts[2].trim().to_string();
        st.baseline = base;
        st.cal_count = cal;
    }
}

// ============================================================
//  BOLD TEXT HELPER & FONT LOADER
// ============================================================
fn configure_bold_fonts(ctx: &egui::Context) {
    let bold_paths = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/ubuntu/Ubuntu-B.ttf",
        "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    ];

    for path in &bold_paths {
        if let Ok(bytes) = fs::read(path) {
            let mut fonts = FontDefinitions::default();
            fonts
            .font_data
            .insert("system_bold".to_owned(), FontData::from_owned(bytes));
            fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "system_bold".to_owned());
            ctx.set_fonts(fonts);
            break;
        }
    }
}

fn draw_bold_text(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl ToString,
    font_id: FontId,
    color: Color32,
) {
    let s = text.to_string();
    painter.text(pos, anchor, &s, font_id.clone(), color);
    painter.text(Pos2::new(pos.x + 0.7, pos.y), anchor, &s, font_id, color);
}

// ============================================================
//  MAIN GUI APPLICATION
// ============================================================
struct VibroWatchApp {
    state: Arc<Mutex<TelemetryState>>,
    // Port Dialog State
    show_port_dialog: bool,
    port_input: String,
    focus_port_input: bool,
    // FPS Controller State
    show_fps_dialog: bool,
    target_fps: u32,
    wave_paused: bool,
    last_wave_update: Instant,
    displayed_wave: Vec<f32>,
    // Plot Navigation State
    tool: ActiveTool,
    view: ViewBounds,
    history: Vec<ViewBounds>,
    history_idx: usize,
    zoom_drag_start: Option<Pos2>,
}

impl VibroWatchApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_bold_fonts(&cc.egui_ctx);

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(13, 17, 23);
        cc.egui_ctx.set_visuals(visuals);

        let state = Arc::new(Mutex::new(TelemetryState::default()));
        spawn_serial_worker(Arc::clone(&state), cc.egui_ctx.clone());

        let default_view = ViewBounds::default();
        Self {
            state,
            show_port_dialog: true,
            port_input: String::new(),
            focus_port_input: true,
            show_fps_dialog: false,
            target_fps: 60,
            wave_paused: false,
            last_wave_update: Instant::now(),
            displayed_wave: vec![CENTER_Y; MAX_POINTS],
            tool: ActiveTool::None,
            view: default_view,
            history: vec![default_view],
            history_idx: 0,
            zoom_drag_start: None,
        }
    }

    fn push_view(&mut self, new_view: ViewBounds) {
        self.history.truncate(self.history_idx + 1);
        self.history.push(new_view);
        self.history_idx = self.history.len() - 1;
        self.view = new_view;
    }

    fn apply_port(&mut self) {
        let trimmed = self.port_input.trim().to_string();
        if !trimmed.is_empty() {
            let mut st = self.state.lock().unwrap();
            st.target_port = Some(trimmed);
            st.connected = false;
            st.port_error = "Connecting...".to_string();
            self.show_port_dialog = false;
        }
    }
}

impl eframe::App for VibroWatchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ============================================================
        //  SINE-WAVE FPS SCHEDULER
        // ============================================================
        let frame_interval = Duration::from_secs_f32(1.0 / (self.target_fps.max(1) as f32));
        if !self.wave_paused && self.last_wave_update.elapsed() >= frame_interval {
            let st = self.state.lock().unwrap();
            self.displayed_wave = st.wave_data.iter().copied().collect();
            self.last_wave_update = Instant::now();
        }
        if !self.wave_paused {
            ctx.request_repaint_after(frame_interval);
        }

        // ============================================================
        //  1. SERIAL PORT SELECTION DIALOG
        // ============================================================
        if self.show_port_dialog {
            egui::Window::new("🔌 Select Serial Port")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(400.0, 210.0))
            .frame(
                egui::Frame::window(&ctx.style())
                .fill(Color32::from_rgb(22, 27, 34))
                .stroke(Stroke::new(2.0, Color32::from_rgb(56, 139, 253)))
                .rounding(Rounding::same(8.0))
                .inner_margin(egui::Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Enter your STM32 / UART Serial Port:")
                    .size(15.5)
                    .strong()
                    .color(Color32::WHITE),
                );
                ui.add_space(10.0);

                let mut autofill_triggered = false;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Quick Autofill:")
                        .size(13.5)
                        .strong()
                        .color(Color32::from_rgb(139, 148, 158)),
                    );

                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("/dev/ttyUSB")
                                .font(FontId::monospace(14.0))
                                .strong()
                                .color(Color32::from_rgb(88, 166, 255)),
                            )
                            .fill(Color32::from_rgb(31, 41, 55)),
                        )
                        .on_hover_text("Click to autofill '/dev/ttyUSB' then type port number")
                        .clicked()
                        {
                            self.port_input = "/dev/ttyUSB".to_string();
                            autofill_triggered = true;
                        }

                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("COM")
                                    .font(FontId::monospace(14.0))
                                    .strong()
                                    .color(Color32::from_rgb(63, 185, 80)),
                                )
                                .fill(Color32::from_rgb(26, 46, 35)),
                            )
                            .on_hover_text("Click to autofill 'COM' then type port number")
                            .clicked()
                            {
                                self.port_input = "COM".to_string();
                                autofill_triggered = true;
                            }
                });

                ui.add_space(10.0);

                let input_id = egui::Id::new("serial_port_input");
                let te_output = egui::TextEdit::singleline(&mut self.port_input)
                .id(input_id)
                .hint_text("e.g. /dev/ttyUSB0 or COM3")
                .font(FontId::monospace(16.5))
                .desired_width(f32::INFINITY)
                .show(ui);

                if self.focus_port_input || autofill_triggered {
                    te_output.response.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, input_id) {
                        let end_pos = CCursor::new(self.port_input.chars().count());
                        state.cursor.set_char_range(Some(CCursorRange::one(end_pos)));
                        state.store(ctx, input_id);
                    }
                    self.focus_port_input = false;
                }

                let enter_pressed = ctx.input(|i| i.key_pressed(egui::Key::Enter))
                && !self.port_input.trim().is_empty();

                ui.add_space(14.0);

                let can_connect = !self.port_input.trim().is_empty();
                ui.vertical_centered(|ui| {
                    let connect_btn = egui::Button::new(
                        egui::RichText::new("🚀 Connect & Launch")
                        .size(15.5)
                        .strong()
                        .color(Color32::WHITE),
                    )
                    .min_size(Vec2::new(220.0, 36.0))
                    .fill(if can_connect {
                        Color32::from_rgb(35, 134, 54)
                    } else {
                        Color32::from_rgb(48, 54, 61)
                    });

                    if (ui.add_enabled(can_connect, connect_btn).clicked()) || enter_pressed {
                        self.apply_port();
                    }
                });
            });
        }

        // ============================================================
        //  2. SINE-WAVE FPS CONTROL DIALOG
        // ============================================================
        if self.show_fps_dialog {
            egui::Window::new("⚡ Sine-Wave FPS Control")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(420.0, 220.0))
            .frame(
                egui::Frame::window(&ctx.style())
                .fill(Color32::from_rgb(22, 27, 34))
                .stroke(Stroke::new(2.0, Color32::from_rgb(57, 255, 20)))
                .rounding(Rounding::same(8.0))
                .inner_margin(egui::Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Adjust Sine-Wave Oscilloscope Refresh Rate:")
                    .size(15.0)
                    .strong()
                    .color(Color32::WHITE),
                );
                ui.add_space(10.0);

                // --- FPS SLIDER (1 - 240 FPS) ---
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Target FPS:")
                        .size(14.0)
                        .strong()
                        .color(Color32::from_rgb(139, 148, 158)),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.target_fps, 1..=240)
                        .suffix(" FPS")
                        .text_color(Color32::from_rgb(57, 255, 20)),
                    );
                });

                ui.add_space(8.0);

                // --- QUICK FPS PRESETS ---
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Presets:")
                        .size(13.5)
                        .strong()
                        .color(Color32::from_rgb(139, 148, 158)),
                    );
                    for &preset in &[10u32, 30, 60, 120, 240] {
                        let is_selected = self.target_fps == preset && !self.wave_paused;
                        let btn = egui::Button::new(
                            egui::RichText::new(format!("{} FPS", preset))
                            .size(13.0)
                            .strong()
                            .color(if is_selected {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(201, 209, 217)
                            }),
                        )
                        .fill(if is_selected {
                            Color32::from_rgb(31, 111, 235)
                        } else {
                            Color32::from_rgb(48, 54, 61)
                        });

                        if ui.add(btn).clicked() {
                            self.target_fps = preset;
                            self.wave_paused = false;
                        }
                    }
                });

                ui.add_space(12.0);

                // --- PAUSE / RESUME & CLOSE BUTTONS ---
                ui.horizontal(|ui| {
                    let pause_label = if self.wave_paused {
                        "▶ Resume Wave"
                    } else {
                        "⏸ Pause Wave"
                    };
                    let pause_btn = egui::Button::new(
                        egui::RichText::new(pause_label)
                        .size(14.0)
                        .strong()
                        .color(Color32::WHITE),
                    )
                    .min_size(Vec2::new(150.0, 32.0))
                    .fill(if self.wave_paused {
                        Color32::from_rgb(210, 123, 0)
                    } else {
                        Color32::from_rgb(48, 54, 61)
                    });

                    if ui.add(pause_btn).clicked() {
                        self.wave_paused = !self.wave_paused;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let done_btn = egui::Button::new(
                            egui::RichText::new("✔ Done")
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                        )
                        .min_size(Vec2::new(110.0, 32.0))
                        .fill(Color32::from_rgb(35, 134, 54));

                        if ui.add(done_btn).clicked() {
                            self.show_fps_dialog = false;
                        }
                    });
                });
            });
        }

        let (freq, status, baseline, cal_count, connected, port_error, active_port) = {
            let st = self.state.lock().unwrap();
            (
                st.freq,
             st.status.clone(),
             st.baseline,
             st.cal_count,
             st.connected,
             st.port_error.clone(),
             st.target_port.clone().unwrap_or_else(|| "None".to_string()),
            )
        };

        // --- TOP MATPLOTLIB-STYLE TOOLBAR ---
        egui::TopBottomPanel::top("toolbar_panel")
        .frame(
            egui::Frame::none()
            .fill(Color32::from_rgb(28, 33, 40))
            .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                // 1. HOME (Reset View)
                if ui
                    .button(egui::RichText::new("🏠 Home").size(14.0).strong())
                    .on_hover_text("Reset view to default (Centered at 512)")
                    .clicked()
                    {
                        self.push_view(ViewBounds::default());
                    }

                    // 2. BACK
                    let can_back = self.history_idx > 0;
                if ui
                    .add_enabled(
                        can_back,
                        egui::Button::new(egui::RichText::new("⬅").size(14.0).strong()),
                    )
                    .on_hover_text("Back to previous view")
                    .clicked()
                    {
                        self.history_idx -= 1;
                        self.view = self.history[self.history_idx];
                    }

                    // 3. FORWARD
                    let can_fwd = self.history_idx + 1 < self.history.len();
                    if ui
                        .add_enabled(
                            can_fwd,
                            egui::Button::new(egui::RichText::new("➡").size(14.0).strong()),
                        )
                        .on_hover_text("Forward to next view")
                        .clicked()
                        {
                            self.history_idx += 1;
                            self.view = self.history[self.history_idx];
                        }

                        ui.separator();

                        // 4. PAN / MOVE BUTTON
                        let pan_active = self.tool == ActiveTool::Pan;
                        let pan_btn = egui::Button::new(
                            egui::RichText::new("✢ Pan / Move")
                            .size(14.0)
                            .strong()
                            .color(if pan_active {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(201, 209, 217)
                            }),
                        )
                        .fill(if pan_active {
                            Color32::from_rgb(31, 111, 235)
                        } else {
                            Color32::from_rgb(48, 54, 61)
                        });

                        if ui
                            .add(pan_btn)
                            .on_hover_text("Drag on the graph to move/pan X & Y axes")
                            .clicked()
                            {
                                self.tool = if pan_active {
                                    ActiveTool::None
                                } else {
                                    ActiveTool::Pan
                                };
                            }

                            // 5. ZOOM BUTTON
                            let zoom_active = self.tool == ActiveTool::Zoom;
                            let zoom_btn = egui::Button::new(
                                egui::RichText::new("🔍 Zoom")
                                .size(14.0)
                                .strong()
                                .color(if zoom_active {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(201, 209, 217)
                                }),
                            )
                            .fill(if zoom_active {
                                Color32::from_rgb(31, 111, 235)
                            } else {
                                Color32::from_rgb(48, 54, 61)
                            });

                            if ui
                                .add(zoom_btn)
                                .on_hover_text("Drag a box on the graph to zoom into a region")
                                .clicked()
                                {
                                    self.tool = if zoom_active {
                                        ActiveTool::None
                                    } else {
                                        ActiveTool::Zoom
                                    };
                                }

                                ui.separator();

                                // 6. SINE-WAVE FPS CONTROL BUTTON
                                let fps_btn_label = if self.wave_paused {
                                    "⚡ FPS: PAUSED".to_string()
                                } else {
                                    format!("⚡ FPS: {}", self.target_fps)
                                };
                                let fps_btn = egui::Button::new(
                                    egui::RichText::new(fps_btn_label)
                                    .size(13.5)
                                    .strong()
                                    .color(if self.wave_paused {
                                        Color32::from_rgb(255, 180, 50)
                                    } else {
                                        Color32::from_rgb(57, 255, 20)
                                    }),
                                )
                                .fill(if self.show_fps_dialog {
                                    Color32::from_rgb(31, 111, 235)
                                } else {
                                    Color32::from_rgb(48, 54, 61)
                                });

                                if ui
                                    .add(fps_btn)
                                    .on_hover_text("Open Sine-Wave FPS Controller")
                                    .clicked()
                                    {
                                        self.show_fps_dialog = !self.show_fps_dialog;
                                    }

                                    // 7. CHANGE PORT BUTTON
                                    if ui
                                        .button(egui::RichText::new("🔌 Change Port").size(13.5).strong())
                                        .on_hover_text("Open Serial Port Selection Dialog")
                                        .clicked()
                                        {
                                            self.show_port_dialog = true;
                                            self.focus_port_input = true;
                                        }

                                        // Right-side Serial Link Status
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if connected {
                                                ui.label(
                                                    egui::RichText::new(format!("● UART LINK ACTIVE ({})", active_port))
                                                    .size(13.5)
                                                    .color(Color32::from_rgb(63, 185, 80))
                                                    .strong(),
                                                );
                                            } else {
                                                ui.label(
                                                    egui::RichText::new(format!("○ {}", port_error))
                                                    .size(13.5)
                                                    .color(Color32::from_rgb(210, 153, 34))
                                                    .strong(),
                                                );
                                            }
                                        });
            });
        });

        // --- MAIN CENTRAL PANEL ---
        egui::CentralPanel::default().show(ctx, |ui| {
            // Centered Bold Title
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("VibroWatch Real-Time Vibration Dashboard")
                    .size(22.0)
                    .strong()
                    .color(Color32::WHITE),
                );
                ui.add_space(4.0);
            });

            // --- OSCILLOSCOPE PLOT AREA ---
            let available = ui.available_size();
            let left_margin = 62.0;
            let bottom_margin = 28.0;

            let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
            let full_rect = response.rect;

            let plot_rect = Rect::from_min_max(
                Pos2::new(full_rect.left() + left_margin, full_rect.top() + 4.0),
                                               Pos2::new(full_rect.right() - 12.0, full_rect.bottom() - bottom_margin),
            );

            let modal_open = self.show_port_dialog || self.show_fps_dialog;
            if !modal_open {
                if response.hovered() {
                    match self.tool {
                        ActiveTool::Pan => ctx.set_cursor_icon(egui::CursorIcon::Grab),
                                           ActiveTool::Zoom => ctx.set_cursor_icon(egui::CursorIcon::Crosshair),
                                           ActiveTool::None => {}
                    }
                }

                // 1. Scroll wheel zoom
                if response.hovered() {
                    let scroll_y = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll_y.abs() > 0.1 {
                        let factor = if scroll_y > 0.0 { 0.88 } else { 1.14 };
                        let y_center = 0.5 * (self.view.y_min + self.view.y_max);
                        let y_half = 0.5 * (self.view.y_max - self.view.y_min) * factor;
                        self.view.y_min = y_center - y_half.clamp(10.0, 2500.0);
                        self.view.y_max = y_center + y_half.clamp(10.0, 2500.0);
                    }
                }

                // 2. Pan / Move Dragging
                if self.tool == ActiveTool::Pan {
                    if response.dragged_by(egui::PointerButton::Primary) {
                        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                        let delta = response.drag_delta();
                        let dx = -delta.x / plot_rect.width() * (self.view.x_max - self.view.x_min);
                        let dy = delta.y / plot_rect.height() * (self.view.y_max - self.view.y_min);
                        self.view.x_min += dx;
                        self.view.x_max += dx;
                        self.view.y_min += dy;
                        self.view.y_max += dy;
                    }
                    if response.drag_stopped() {
                        let v = self.view;
                        self.push_view(v);
                    }
                }

                // 3. Box-Zoom Dragging
                if self.tool == ActiveTool::Zoom {
                    if response.drag_started_by(egui::PointerButton::Primary) {
                        self.zoom_drag_start = response.interact_pointer_pos();
                    }
                    if response.drag_stopped() {
                        if let (Some(start), Some(end)) =
                            (self.zoom_drag_start, response.interact_pointer_pos())
                            {
                                let sel = Rect::from_two_pos(start, end).intersect(plot_rect);
                                if sel.width() > 10.0 && sel.height() > 10.0 {
                                    let x0_norm = (sel.left() - plot_rect.left()) / plot_rect.width();
                                    let x1_norm = (sel.right() - plot_rect.left()) / plot_rect.width();
                                    let y0_norm =
                                    (plot_rect.bottom() - sel.bottom()) / plot_rect.height();
                                    let y1_norm =
                                    (plot_rect.bottom() - sel.top()) / plot_rect.height();

                                    let x_span = self.view.x_max - self.view.x_min;
                                    let y_span = self.view.y_max - self.view.y_min;

                                    let new_view = ViewBounds {
                                        x_min: self.view.x_min + x0_norm * x_span,
                                        x_max: self.view.x_min + x1_norm * x_span,
                                        y_min: self.view.y_min + y0_norm * y_span,
                                        y_max: self.view.y_min + y1_norm * y_span,
                                    };
                                    self.push_view(new_view);
                                }
                            }
                            self.zoom_drag_start = None;
                    }
                }
            }

            // Plot background (#161b22) & border (#30363d)
            painter.rect(
                plot_rect,
                Rounding::same(4.0),
                         Color32::from_rgb(22, 27, 34),
                         Stroke::new(1.5, Color32::from_rgb(48, 54, 61)),
            );

            // Coordinate Mappers
            let v = self.view;
            let map_y = |val: f32| -> f32 {
                let norm = (val - v.y_min) / (v.y_max - v.y_min).max(1e-3);
                plot_rect.bottom() - norm * plot_rect.height()
            };
            let map_x = |idx: f32| -> f32 {
                let norm = (idx - v.x_min) / (v.x_max - v.x_min).max(1e-3);
                plot_rect.left() + norm * plot_rect.width()
            };

            let plot_painter = painter.with_clip_rect(plot_rect);

            // --- DYNAMIC HORIZONTAL GRID LINES & BOLD Y-AXIS LABELS ---
            let num_y_steps = 6;
            for i in 0..=num_y_steps {
                let frac = i as f32 / num_y_steps as f32;
                let tick_val = v.y_min + frac * (v.y_max - v.y_min);
                let y = map_y(tick_val);

                plot_painter.line_segment(
                    [Pos2::new(plot_rect.left(), y), Pos2::new(plot_rect.right(), y)],
                                          Stroke::new(0.8, Color32::from_rgba_premultiplied(48, 54, 61, 140)),
                );

                draw_bold_text(
                    &painter,
                    Pos2::new(plot_rect.left() - 8.0, y),
                               Align2::RIGHT_CENTER,
                               format!("{:.0}", tick_val),
                                   FontId::monospace(12.5),
                               Color32::from_rgb(139, 148, 158),
                );
            }

            // Highlighted DC Offset Reference Line at 512
            if CENTER_Y >= v.y_min && CENTER_Y <= v.y_max {
                let y512 = map_y(CENTER_Y);
                plot_painter.line_segment(
                    [
                        Pos2::new(plot_rect.left(), y512),
                                          Pos2::new(plot_rect.right(), y512),
                    ],
                    Stroke::new(1.5, Color32::from_rgb(56, 139, 253)),
                );
            }

            // --- DYNAMIC VERTICAL GRID LINES & BOLD X-AXIS LABELS ---
            let num_x_steps = 8;
            for i in 0..=num_x_steps {
                let frac = i as f32 / num_x_steps as f32;
                let tick_x = v.x_min + frac * (v.x_max - v.x_min);
                let x = map_x(tick_x);

                plot_painter.line_segment(
                    [Pos2::new(x, plot_rect.top()), Pos2::new(x, plot_rect.bottom())],
                                          Stroke::new(0.8, Color32::from_rgba_premultiplied(48, 54, 61, 120)),
                );

                draw_bold_text(
                    &painter,
                    Pos2::new(x, plot_rect.bottom() + 14.0),
                               Align2::CENTER_CENTER,
                               format!("{:.0}", tick_x),
                                   FontId::monospace(12.0),
                               Color32::from_rgb(139, 148, 158),
                );
            }

            // --- WAVEFORM LINE (#39ff14 Neon Green, Controlled by Target FPS) ---
            if self.displayed_wave.len() > 1 {
                let points: Vec<Pos2> = self
                .displayed_wave
                .iter()
                .enumerate()
                .map(|(i, &val)| Pos2::new(map_x(i as f32), map_y(val)))
                .collect();

                plot_painter.add(PathShape::line(
                    points,
                    Stroke::new(2.4, Color32::from_rgb(57, 255, 20)),
                ));
            }

            // Draw active Box-Zoom selection rectangle
            if !modal_open
                && self.tool == ActiveTool::Zoom
                && response.dragged_by(egui::PointerButton::Primary)
                {
                    if let (Some(start), Some(curr)) =
                        (self.zoom_drag_start, response.interact_pointer_pos())
                        {
                            let sel_rect = Rect::from_two_pos(start, curr).intersect(plot_rect);
                            plot_painter.rect(
                                sel_rect,
                                Rounding::same(2.0),
                                              Color32::from_rgba_unmultiplied(88, 166, 255, 45),
                                              Stroke::new(1.5, Color32::from_rgb(88, 166, 255)),
                            );
                        }
                }

                // --- OVERLAY STATUS BANNER & BOLD TELEMETRY (Top-Left of Plot) ---
                let (banner_text, fill_col, edge_col) = match status.as_str() {
                    "CALIBRATING" | "CAL" => (
                        format!("CALIBRATING: STRIKE THE PLATE ({}/5)", cal_count),
                            Color32::from_rgb(31, 58, 95),
                                              Color32::from_rgb(46, 92, 158),
                    ),
                    "READY" => {
                        if cal_count >= 5 && baseline > 0.0 {
                            (
                                format!(
                                    "CALIBRATION COMPLETE — BASELINE {:.1} Hz LOCKED",
                                    baseline
                                ),
                             Color32::from_rgb(26, 58, 42),
                             Color32::from_rgb(46, 160, 67),
                            )
                        } else {
                            (
                                "SYSTEM STATUS: IDLE (WAITING FOR IMPACT)".to_string(),
                             Color32::from_rgb(31, 36, 40),
                             Color32::from_rgb(48, 54, 61),
                            )
                        }
                    }
                    "MATCHED" => (
                        "STATUS: FREQUENCY MATCHED (NORMAL)".to_string(),
                                  Color32::from_rgb(35, 134, 54),
                                  Color32::from_rgb(46, 160, 67),
                    ),
                    "NEW" => (
                        "STATUS: NEW FREQUENCY DETECTED (ANOMALY)".to_string(),
                              Color32::from_rgb(218, 54, 51),
                              Color32::from_rgb(248, 81, 73),
                    ),
                    "TOO_HARD" => (
                        "ERROR: HEAVY IMPACT — PLEASE TAP GENTLY".to_string(),
                                   Color32::from_rgb(210, 123, 0),
                                   Color32::from_rgb(224, 144, 0),
                    ),
                    _ => (
                        "SYSTEM STATUS: IDLE (WAITING FOR IMPACT)".to_string(),
                          Color32::from_rgb(31, 36, 40),
                          Color32::from_rgb(48, 54, 61),
                    ),
                };

                let box_min = Pos2::new(plot_rect.left() + 16.0, plot_rect.top() + 16.0);
                let box_rect = Rect::from_min_size(box_min, Vec2::new(515.0, 38.0));
                painter.rect(
                    box_rect,
                    Rounding::same(6.0),
                             fill_col,
                             Stroke::new(1.8, edge_col),
                );
                draw_bold_text(
                    &painter,
                    Pos2::new(box_rect.left() + 14.0, box_rect.center().y),
                               Align2::LEFT_CENTER,
                               banner_text,
                               FontId::proportional(15.5),
                               Color32::WHITE,
                );

                // Bold Resonance Frequency Label
                let freq_str = if freq > 0.0 {
                    format!("Resonance Frequency: {:.1} Hz", freq)
                } else {
                    "Resonance Frequency: -- Hz".to_string()
                };
                draw_bold_text(
                    &painter,
                    Pos2::new(box_min.x + 2.0, box_rect.bottom() + 24.0),
                               Align2::LEFT_CENTER,
                               freq_str,
                               FontId::proportional(17.0),
                               Color32::from_rgb(88, 166, 255),
                );

                // Bold Baseline Label
                let (base_str, base_col) = if cal_count < 5 {
                    (
                        format!("Baseline: Calibrating... ({}/5 strikes)", cal_count),
                            Color32::from_rgb(139, 148, 158),
                    )
                } else {
                    (
                        format!("Baseline: {:.1} Hz  (LOCKED)", baseline),
                            Color32::from_rgb(63, 185, 80),
                    )
                };
                draw_bold_text(
                    &painter,
                    Pos2::new(box_min.x + 2.0, box_rect.bottom() + 50.0),
                               Align2::LEFT_CENTER,
                               base_str,
                               FontId::proportional(15.5),
                               base_col,
                );
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
        .with_inner_size([1150.0, 700.0])
        .with_title("VibroWatch Sine-wave Observation Dashboard"),
        ..Default::default()
    };

    eframe::run_native(
        "VibroWatch Sine-wave Observation Dashboard",
        options,
        Box::new(|cc| Box::new(VibroWatchApp::new(cc))),
    )
}
