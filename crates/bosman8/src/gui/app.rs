//! The egui window.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Ui};

use bosman8::disk;

use super::config::{AppConfig, LinkCfg, LinkKind, PrinterKind};
use super::emu::{Cmd, Emu, LinkStatus, Status};

const BAUDS: [u32; 11] = [300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400];
const DRIVE_NAMES: [&str; 2] = ["B:", "C:"];
const GREEN: Color32 = Color32::from_rgb(40, 200, 60);
const RED: Color32 = Color32::from_rgb(230, 50, 40);
const WARN: Color32 = Color32::from_rgb(230, 160, 30);

pub struct App {
    cfg: AppConfig,
    config_path: Option<PathBuf>,
    saved_cfg: AppConfig,
    last_save: Instant,
    emu: Option<Emu>,
    rom_error: Option<String>,
    rom_path: Option<PathBuf>,
    serial_ports: Vec<String>,
    /// Size (KB) chosen for "New blank" per drive.
    new_kb: [usize; 2],
    /// Local copy of the last status (the lock is held only to copy it).
    st: Status,
    messages: Vec<String>,
}

impl App {
    pub fn new(cfg: AppConfig, config_path: Option<PathBuf>) -> App {
        let new_kb = [0, 1].map(|d| disk::format_for(cfg.dip, d).kbytes());
        let mut app = App {
            saved_cfg: cfg.clone(),
            cfg,
            config_path,
            last_save: Instant::now(),
            emu: None,
            rom_error: None,
            rom_path: None,
            serial_ports: list_serial_ports(),
            new_kb,
            st: Status::default(),
            messages: Vec::new(),
        };
        app.start();
        app
    }

    /// (Re)start the machine from the current settings.
    fn start(&mut self) {
        self.emu = None; // stops the old thread, writes the disks back
        match super::super::find_rom(self.cfg.rom.as_deref()) {
            Ok((path, rom)) => {
                self.rom_error = None;
                self.rom_path = Some(path);
                self.emu = Some(Emu::start(rom, &self.cfg));
            }
            Err(e) => self.rom_error = Some(e),
        }
    }

    /// Command sender usable while parts of `self` are borrowed.
    fn tx(&self) -> Tx {
        Tx(self.emu.as_ref().map(Emu::sender))
    }

    fn save_config(&mut self, force: bool) {
        if self.cfg == self.saved_cfg || (!force && self.last_save.elapsed() < Duration::from_secs(1)) {
            return;
        }
        if let Some(p) = &self.config_path {
            if let Err(e) = self.cfg.save(p) {
                self.messages.push(format!("config: {e}"));
            }
        }
        self.saved_cfg = self.cfg.clone();
        self.last_save = Instant::now();
    }

    fn machine_panel(&mut self, ui: &mut Ui) {
        let tx = self.tx();
        let st = &self.st;
        ui.horizontal_wrapped(|ui| {
            let run_label = if st.running { "⏸ Stop" } else { "▶ Start" };
            if ui.button(run_label).on_hover_text("Wstrzymaj / wznów emulację").clicked() {
                tx.send(Cmd::Run(!st.running));
            }
            if ui
                .button(RichText::new("RESET").strong())
                .on_hover_text(
                    "Przycisk RESET: zerowany tylko procesor, RAM i zatrzaski zostają \
                     (program w RAM → ciepły start CP/M, sam znak zachęty)",
                )
                .clicked()
            {
                tx.send(Cmd::WarmReset);
            }
            if ui
                .button("Wyłącz/włącz")
                .on_hover_text("Zimny start jak po włączeniu zasilania: RAM i zatrzaski wyzerowane, zworki DIP zastosowane, baner")
                .clicked()
            {
                tx.send(Cmd::ColdReset(self.cfg.dip));
            }
            if ui.button("NMI").on_hover_text("Impuls na linii NMI (eksperymentalnie)").clicked() {
                tx.send(Cmd::Nmi);
            }
            if ui.checkbox(&mut self.cfg.turbo, "Turbo").on_hover_text("Bez synchronizacji z zegarem, szybkie FDC").changed() {
                tx.send(Cmd::Turbo(self.cfg.turbo));
            }
        });
        let st = &self.st;
        let up = st.uptime as u64;
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!(
                "{:6.2} MHz ({:4.0}%)  czas {:02}:{:02}:{:02}  PC={:04X}  zatrzask {:02X} (bank {}, {})  zatrzask2 {:02X}  DIP {:02X}",
                st.mhz,
                st.mhz / 4.0 * 100.0,
                up / 3600,
                up / 60 % 60,
                up % 60,
                st.pc,
                st.latch,
                st.bank,
                if st.ram_at_0 { "RAM" } else { "ROM" },
                st.latch2,
                st.dip,
            ));
            if !st.running {
                ui.colored_label(WARN, "wstrzymany");
            }
        });
    }

    fn dip_panel(&mut self, ui: &mut Ui) {
        let bit = |ui: &mut Ui, cfg: &mut AppConfig, b: u8, label: &str| {
            let mut on = cfg.dip & (1 << b) != 0;
            if ui.checkbox(&mut on, label).changed() {
                cfg.dip = if on { cfg.dip | 1 << b } else { cfg.dip & !(1 << b) };
            }
        };
        ui.horizontal_wrapped(|ui| {
            bit(ui, &mut self.cfg, 3, "bit 3 (format)");
            bit(ui, &mut self.cfg, 5, "bit 5 (format)");
            bit(ui, &mut self.cfg, 6, "bit 6 (1 = brak napędu C:)");
            bit(ui, &mut self.cfg, 1, "bit 1 (?)");
        });
        let d = self.cfg.dip;
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "DIP {:02X}: dyskietki B: {} KB, C: {}",
                d,
                disk::format_for(d, 0).kbytes(),
                if d & 0x40 != 0 { "brak".to_string() } else { format!("{} KB", disk::format_for(d, 1).kbytes()) }
            ));
            if d & bosman8::p98::DIP_MASK != self.st.dip & bosman8::p98::DIP_MASK && self.emu.is_some() {
                ui.colored_label(WARN, "— zadziała po „Wyłącz/włącz”");
            }
        });
    }

    fn drive_panel(&mut self, ui: &mut Ui, d: usize) {
        let tx = self.tx();
        let ds = self.st.drives[d].clone();
        let dip = self.st.dip;
        let expected = disk::format_for(dip, d);
        ui.horizontal(|ui| {
            led(ui, ds.active, GREEN);
            ui.label(RichText::new(DRIVE_NAMES[d]).strong().monospace());
            match &ds.path {
                Some(p) if ds.inserted => {
                    let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                    ui.label(name).on_hover_text(p.display().to_string());
                }
                _ => {
                    ui.weak("(pusty)");
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Włóż…").clicked() {
                if let Some(p) = image_dialog(self.cfg.drives[d].path.as_deref()).pick_file() {
                    tx.send(Cmd::Insert { drive: d, path: p.clone(), write_protect: self.cfg.drives[d].write_protect });
                    self.cfg.drives[d].path = Some(p);
                }
            }
            if ui.add_enabled(ds.inserted, egui::Button::new("Wysuń")).clicked() {
                tx.send(Cmd::Eject(d));
                self.cfg.drives[d].path = None;
            }
            egui::ComboBox::from_id_salt(("newkb", d))
                .width(70.0)
                .selected_text(format!("{} KB", self.new_kb[d]))
                .show_ui(ui, |ui| {
                    for kb in [200, 400, 800] {
                        ui.selectable_value(&mut self.new_kb[d], kb, format!("{kb} KB"));
                    }
                });
            if ui.button("Nowa…").on_hover_text("Utwórz pustą sformatowaną dyskietkę i włóż ją").clicked() {
                let f = match self.new_kb[d] {
                    800 => disk::F800,
                    400 => disk::F400,
                    _ => disk::F200,
                };
                let dlg = image_dialog(self.cfg.drives[d].path.as_deref())
                    .set_file_name(format!("dysk{}.img", DRIVE_NAMES[d].trim_end_matches(':').to_lowercase()));
                if let Some(p) = dlg.save_file() {
                    match std::fs::write(&p, disk::blank_file(&p, f)) {
                        Ok(()) => {
                            tx.send(Cmd::Insert { drive: d, path: p.clone(), write_protect: false });
                            self.cfg.drives[d].path = Some(p);
                            self.cfg.drives[d].write_protect = false;
                        }
                        Err(e) => self.messages.push(format!("{}: {e}", p.display())),
                    }
                }
            }
            let mut wp = if ds.inserted { ds.write_protect } else { self.cfg.drives[d].write_protect };
            if ui.checkbox(&mut wp, "Zabezpieczona przed zapisem").changed() {
                self.cfg.drives[d].write_protect = wp;
                tx.send(Cmd::WriteProtect(d, wp));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("format wg DIP: {} KB ({} B)", expected.kbytes(), expected.image_len()));
            if ds.inserted && ds.image_len != expected.image_len() {
                ui.colored_label(WARN, format!("⚠ obraz ma {} B", ds.image_len));
            }
            if d == 1 && dip & 0x40 != 0 {
                ui.colored_label(WARN, "⚠ DIP bit 6 = 1: ROM nie używa napędu C:");
            }
        });
    }

    fn link_panel(&mut self, ui: &mut Ui, aux: bool) {
        let tx = self.tx();
        let ls: LinkStatus = if aux { self.st.aux.clone() } else { self.st.term.clone() };
        let ports = self.serial_ports.clone();
        let cfg: &mut LinkCfg = if aux { &mut self.cfg.aux } else { &mut self.cfg.term };
        let mut apply = false;
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("linkkind", aux))
                .selected_text(kind_name(cfg.kind))
                .show_ui(ui, |ui| {
                    for k in [LinkKind::None, LinkKind::TcpListen, LinkKind::TcpConnect, LinkKind::Serial, LinkKind::Spec] {
                        ui.selectable_value(&mut cfg.kind, k, kind_name(k));
                    }
                });
            match cfg.kind {
                LinkKind::None => {}
                LinkKind::TcpListen => {
                    ui.label("adres:port");
                    ui.add(egui::TextEdit::singleline(&mut cfg.listen).desired_width(150.0));
                }
                LinkKind::TcpConnect => {
                    ui.label("host:port");
                    ui.add(egui::TextEdit::singleline(&mut cfg.connect).desired_width(150.0));
                }
                LinkKind::Serial => {
                    egui::ComboBox::from_id_salt(("ports", aux))
                        .selected_text(if cfg.serial_dev.is_empty() { "(port)" } else { cfg.serial_dev.as_str() })
                        .show_ui(ui, |ui| {
                            if ports.is_empty() {
                                ui.weak("brak portów");
                            }
                            for p in &ports {
                                ui.selectable_value(&mut cfg.serial_dev, p.clone(), p);
                            }
                        });
                    ui.add(egui::TextEdit::singleline(&mut cfg.serial_dev).desired_width(120.0).hint_text("/dev/ttyUSB0"));
                    if ui.small_button("⟳").on_hover_text("Odśwież listę portów").clicked() {
                        refresh = true;
                    }
                    egui::ComboBox::from_id_salt(("baud", aux))
                        .width(80.0)
                        .selected_text(format!("{} bd", cfg.baud))
                        .show_ui(ui, |ui| {
                            for b in BAUDS {
                                ui.selectable_value(&mut cfg.baud, b, b.to_string());
                            }
                        });
                }
                LinkKind::Spec => {
                    ui.add(egui::TextEdit::singleline(&mut cfg.spec).desired_width(200.0))
                        .on_hover_text("none | loopback | stdio | tcp-listen:ADDR:PORT | tcp:HOST:PORT | serial:DEV@BAUD");
                }
            }
            if ui.button("Zastosuj").on_hover_text(cfg.spec()).clicked() {
                apply = true;
            }
        });
        if apply {
            let spec = cfg.spec();
            tx.send(Cmd::Link { aux, spec });
        }
        if refresh {
            self.serial_ports = list_serial_ports();
        }
        ui.horizontal_wrapped(|ui| {
            led(ui, ls.connected && ls.error.is_none(), GREEN);
            match &ls.error {
                Some(e) => ui.colored_label(RED, e),
                None => ui.weak(&ls.describe),
            };
        });
    }

    fn printer_panel(&mut self, ui: &mut Ui) {
        let tx = self.tx();
        let open = self.st.printer.is_some();
        let p = &mut self.cfg.printer;
        let mut toggle = false;
        ui.add_enabled_ui(!open, |ui| {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("printerkind")
                    .selected_text(printer_name(p.kind))
                    .show_ui(ui, |ui| {
                        for k in [PrinterKind::None, PrinterKind::File, PrinterKind::Link, PrinterKind::Device] {
                            ui.selectable_value(&mut p.kind, k, printer_name(k));
                        }
                    });
                match p.kind {
                    PrinterKind::None => {}
                    PrinterKind::File => {
                        if ui.button("Plik…").clicked() {
                            let mut dlg = rfd::FileDialog::new().add_filter("Tekst", &["txt", "prn"]);
                            if let Some(f) = &p.file {
                                if let Some(dir) = f.parent() {
                                    dlg = dlg.set_directory(dir);
                                }
                                if let Some(n) = f.file_name() {
                                    dlg = dlg.set_file_name(n.to_string_lossy());
                                }
                            } else {
                                dlg = dlg.set_file_name("drukarka.txt");
                            }
                            if let Some(f) = dlg.save_file() {
                                p.file = Some(f);
                            }
                        }
                        ui.label(p.file.as_ref().map_or("(nie wybrano)".into(), |f| f.display().to_string()));
                        ui.radio_value(&mut p.append, true, "dopisuj");
                        ui.radio_value(&mut p.append, false, "nadpisuj");
                    }
                    PrinterKind::Link => {
                        ui.add(egui::TextEdit::singleline(&mut p.link).desired_width(220.0))
                            .on_hover_text("tcp:HOST:PORT | tcp-listen:ADDR:PORT | serial:DEV@BAUD");
                    }
                    PrinterKind::Device => {
                        ui.add(egui::TextEdit::singleline(&mut p.device).desired_width(160.0))
                            .on_hover_text("/dev/usb/lp0, /dev/lp0, LPT1 …");
                    }
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let label = if open { "Zamknij" } else { "Otwórz" };
            if ui.add_enabled(open || p.kind != PrinterKind::None, egui::Button::new(label)).clicked() {
                toggle = true;
            }
            led(ui, open, GREEN);
            match (&self.st.printer, &self.st.printer_error) {
                (Some(d), _) => ui.weak(format!("otwarte: {d}")),
                (None, Some(e)) => ui.colored_label(RED, e),
                (None, None) => ui.weak("zamknięte (bajty są odrzucane)"),
            };
            ui.label(format!("wydrukowano: {} B", self.st.printed));
        });
        if toggle {
            if open {
                self.cfg.printer.open = false;
                tx.send(Cmd::Printer(None));
            } else {
                self.cfg.printer.open = true;
                tx.send(Cmd::Printer(Some(self.cfg.printer.clone())));
            }
        }
    }

    fn monitor_panel(&mut self, ui: &mut Ui) {
        let text = monitor_text(self.st.monitor.iter().copied());
        egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(e) = &self.emu {
            let mut s = e.status();
            self.messages.append(&mut s.messages);
            self.st = s.clone();
        }
        if self.messages.len() > 100 {
            let n = self.messages.len() - 100;
            self.messages.drain(..n);
        }

        egui::TopBottomPanel::bottom("messages").resizable(true).default_height(70.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.weak("Komunikaty");
                if ui.small_button("wyczyść").clicked() {
                    self.messages.clear();
                }
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for m in &self.messages {
                    ui.label(RichText::new(m).small());
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(e) = self.rom_error.clone() {
                    ui.colored_label(RED, format!("Brak ROM-u: {e}"));
                    if ui.button("Wybierz plik ROM…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("ROM", &["bin", "rom"]).pick_file() {
                            self.cfg.rom = Some(p);
                            self.start();
                        }
                    }
                    ui.separator();
                }
                section(ui, "Komputer", |ui| self.machine_panel(ui));
                section(ui, "Stacje dyskietek", |ui| {
                    self.drive_panel(ui, 0);
                    ui.separator();
                    self.drive_panel(ui, 1);
                });
                section(ui, "Zworki portu 0x98 (DIP)", |ui| self.dip_panel(ui));
                section(ui, "SIO-A — terminal", |ui| self.link_panel(ui, false));
                section(ui, "SIO-B — V.24", |ui| self.link_panel(ui, true));
                section(ui, "Port równoległy (Centronics)", |ui| self.printer_panel(ui));
                let r = egui::CollapsingHeader::new("Podgląd wyjścia SIO-A")
                    .default_open(self.cfg.show_monitor)
                    .show(ui, |ui| self.monitor_panel(ui));
                self.cfg.show_monitor = r.body_returned.is_some();
                if let Some(p) = &self.rom_path {
                    ui.weak(format!("ROM: {}", p.display()));
                }
            });
        });

        self.save_config(false);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.emu = None; // write the disks back
        self.save_config(true);
    }
}

struct Tx(Option<std::sync::mpsc::Sender<Cmd>>);

impl Tx {
    fn send(&self, c: Cmd) {
        if let Some(t) = &self.0 {
            let _ = t.send(c);
        }
    }
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).strong());
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        body(ui)
    });
}

fn led(ui: &mut Ui, on: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    let c = if on { color } else { Color32::from_gray(70) };
    ui.painter().circle_filled(rect.center(), 5.0, c);
}

fn image_dialog(current: Option<&Path>) -> rfd::FileDialog {
    let mut dlg = rfd::FileDialog::new().add_filter("Obraz dyskietki", &["img", "dsk", "raw", "hfe"]).add_filter("Wszystkie", &["*"]);
    if let Some(dir) = current.and_then(Path::parent).filter(|d| d.is_dir()) {
        dlg = dlg.set_directory(dir);
    } else if Path::new("disks").is_dir() {
        if let Ok(d) = std::fs::canonicalize("disks") {
            dlg = dlg.set_directory(d);
        }
    }
    dlg
}

fn kind_name(k: LinkKind) -> &'static str {
    match k {
        LinkKind::None => "brak",
        LinkKind::TcpListen => "TCP — nasłuch",
        LinkKind::TcpConnect => "TCP — połączenie",
        LinkKind::Serial => "port szeregowy hosta",
        LinkKind::Spec => "inna specyfikacja",
    }
}

fn printer_name(k: PrinterKind) -> &'static str {
    match k {
        PrinterKind::None => "brak",
        PrinterKind::File => "plik",
        PrinterKind::Link => "łącze (TCP / port szeregowy)",
        PrinterKind::Device => "urządzenie hosta",
    }
}

fn list_serial_ports() -> Vec<String> {
    serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

/// Raw terminal output as text: CR dropped, ESC shown as `^[`, other control
/// characters as `·`.
fn monitor_text(bytes: impl Iterator<Item = u8>) -> String {
    let mut s = String::new();
    for b in bytes {
        match b {
            b'\n' => s.push('\n'),
            b'\r' => {}
            0x1B => s.push_str("^["),
            0x20..=0x7E => s.push(b as char),
            _ => s.push('·'),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn monitor_text() {
        assert_eq!(super::monitor_text(b"A>\x1b=\r\n\x01x".iter().copied()), "A>^[=\n·x");
    }
}
