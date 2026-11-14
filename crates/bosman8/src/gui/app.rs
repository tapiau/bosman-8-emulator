//! The egui window, shared by the modes: the terminal screen in the centre
//! ("full", "terminal"), settings in a side panel that can be hidden,
//! the terminal's buttons and the drive lamps in the bottom bar.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Ui};

use b8_link::{Endpoint, NullEndpoint};
use bosman8::disk;

use super::config::{AppConfig, LinkCfg, LinkKind, Mode, PrinterKind};
use super::emu::{Cmd, Emu, LinkStatus, Status};
use super::i18n::{self, t, Lang};
use super::terminal::{lamp, FAction, TCmd, TermEmu, TermView};
use crate::tf;

const BAUDS: [u32; 11] = [300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400];
const DRIVE_NAMES: [&str; 2] = ["B:", "C:"];
const GREEN: Color32 = Color32::from_rgb(40, 200, 60);
const RED: Color32 = Color32::from_rgb(230, 50, 40);
const WARN: Color32 = Color32::from_rgb(230, 160, 30);

pub struct App {
    mode: Mode,
    cfg: AppConfig,
    config_path: Option<PathBuf>,
    saved_cfg: AppConfig,
    last_save: Instant,
    /// The computer (modes "full", "computer").
    emu: Option<Emu>,
    /// The terminal (modes "full", "terminal").
    term: Option<TermEmu>,
    view: TermView,
    term_opts: TermOpts,
    term_rom_desc: String,
    rom_error: Option<String>,
    rom_path: Option<PathBuf>,
    serial_ports: Vec<String>,
    /// Size (KB) chosen for "New blank" per drive.
    new_kb: [usize; 2],
    /// Local copy of the last status (the lock is held only to copy it).
    st: Status,
    messages: Vec<String>,
}

/// Terminal settings that come only from the command line.
#[derive(Clone, Default)]
pub struct TermOpts {
    /// Clocks, character time, `--fixed-baud` (colour and line offset come from the settings).
    pub base: ang3001::Config,
    /// `--type` text (typed once after the first boot).
    pub typed: Vec<u8>,
    /// `--inject` / `--load-ter` bytes (delivered once after the first boot).
    pub inject: Vec<u8>,
}

/// Which serial line a link panel edits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    SioA,
    SioB,
    /// The terminal's own line (mode "terminal").
    Term,
}

impl App {
    pub fn new(mode: Mode, cfg: AppConfig, config_path: Option<PathBuf>, term_opts: TermOpts) -> App {
        let new_kb = [0, 1].map(|d| disk::format_for(cfg.dip, d).kbytes());
        let view = TermView::new(cfg.terminal.arrows());
        let mut app = App {
            mode,
            saved_cfg: cfg.clone(),
            cfg,
            config_path,
            last_save: Instant::now(),
            emu: None,
            term: None,
            view,
            term_opts,
            term_rom_desc: String::new(),
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

    /// (Re)start everything the mode runs.
    fn start(&mut self) {
        self.term = None;
        self.start_computer();
        self.start_terminal();
    }

    /// (Re)start the computer from the current settings.
    fn start_computer(&mut self) {
        self.emu = None; // stops the old thread, writes the disks back
        if !self.mode.has_computer() {
            return;
        }
        match super::super::find_rom(self.cfg.rom.as_deref()) {
            Ok((path, rom)) => {
                self.rom_error = None;
                self.rom_path = Some(path);
                self.emu = Some(Emu::start(rom, &self.cfg));
            }
            Err(e) => self.rom_error = Some(e),
        }
        // A restarted computer needs a new connection to the built-in terminal.
        if self.term.is_some() && self.builtin_terminal() {
            let link = self.terminal_link();
            if let Some(t) = &self.term {
                t.send(TCmd::Link(link));
            }
        }
    }

    /// Command to the computer thread (if the mode has one).
    fn send(&self, c: Cmd) {
        if let Some(e) = &self.emu {
            let _ = e.sender().send(c);
        }
    }

    /// SIO-A goes to the terminal in this window.
    fn builtin_terminal(&self) -> bool {
        self.mode == Mode::Full && self.cfg.term.kind == LinkKind::Terminal
    }

    /// The terminal shows on screen (its line is connected to something here or outside).
    fn screen_shown(&self) -> bool {
        self.term.is_some() && (self.mode == Mode::Terminal || self.builtin_terminal())
    }

    /// (Re)start the terminal.
    fn start_terminal(&mut self) {
        self.term = None;
        if !self.mode.has_terminal() {
            return;
        }
        let rom = self.terminal_rom();
        let mut c = self.term_opts.base.clone();
        c.fg = self.cfg.terminal.fg();
        c.line_offset = self.cfg.terminal.line_offset;
        let link = self.terminal_link();
        let t = TermEmu::start(rom, c, link, self.cfg.turbo);
        let typed = std::mem::take(&mut self.term_opts.typed);
        if !typed.is_empty() {
            t.send(TCmd::Type(typed));
        }
        let inject = std::mem::take(&mut self.term_opts.inject);
        if !inject.is_empty() {
            t.send(TCmd::Inject(inject));
        }
        self.term = Some(t);
    }

    /// The terminal's line: an in-memory pair with the computer ("full"),
    /// nothing ("full" with SIO-A elsewhere) or the configured link ("terminal").
    fn terminal_link(&mut self) -> Box<dyn Endpoint> {
        match self.mode {
            Mode::Full if self.builtin_terminal() => {
                let (a, b) = b8_link::pair();
                if let Some(e) = &self.emu {
                    let _ = e.sender().send(Cmd::TermEndpoint(a));
                }
                b
            }
            Mode::Full | Mode::Computer => Box::new(NullEndpoint),
            Mode::Terminal => match b8_link::open(&self.cfg.terminal.link.spec()) {
                Ok(l) => l,
                Err(e) => {
                    self.messages.push(format!("terminal: {e}"));
                    Box::new(NullEndpoint)
                }
            },
        }
    }

    /// Terminal ROM: settings, else /etc/bosman-8-emulator.conf, else built in.
    fn terminal_rom(&mut self) -> Vec<u8> {
        let path = self.cfg.terminal.rom.clone().or_else(|| b8_link::sysconf::get_path("ang3001_rom"));
        if let Some(p) = path {
            match std::fs::read(&p) {
                Ok(r) => {
                    self.term_rom_desc = p.display().to_string();
                    return r;
                }
                Err(e) => self.messages.push(tf!(
                    "terminal ROM {}: {e} — using the built-in one",
                    "ROM terminala {}: {e} — używam wbudowanego",
                    p.display()
                )),
            }
        }
        self.term_rom_desc.clear(); // built-in, described in the current language when drawn
        ang3001::DEFAULT_ROM.to_vec()
    }

    /// F-key / bar action of the terminal.
    fn apply(&mut self, a: FAction) {
        if a == FAction::Turbo {
            self.cfg.turbo = !self.cfg.turbo;
            self.send(Cmd::Turbo(self.cfg.turbo));
            if let Some(t) = &self.term {
                t.send(TCmd::Turbo(self.cfg.turbo));
            }
        } else if let Some(t) = &self.term {
            self.view.apply(a, t);
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
            if ui.button(run_label).on_hover_text(t("Pause / resume the emulation", "Wstrzymaj / wznów emulację")).clicked() {
                tx.send(Cmd::Run(!st.running));
            }
            if ui
                .button(RichText::new("RESET").strong())
                .on_hover_text(t(
                    "RESET button: only the CPU is reset, RAM and latches stay \
                     (a program in RAM → CP/M warm start, just the prompt)",
                    "Przycisk RESET: zerowany tylko procesor, RAM i zatrzaski zostają \
                     (program w RAM → ciepły start CP/M, sam znak zachęty)",
                ))
                .clicked()
            {
                tx.send(Cmd::WarmReset);
            }
            if ui
                .button(t("Power cycle", "Wyłącz/włącz"))
                .on_hover_text(t(
                    "Cold start as after switching on: RAM and latches cleared, DIP jumpers applied, banner",
                    "Zimny start jak po włączeniu zasilania: RAM i zatrzaski wyzerowane, zworki DIP zastosowane, baner",
                ))
                .clicked()
            {
                tx.send(Cmd::ColdReset(self.cfg.dip));
            }
            if ui.button("NMI").on_hover_text(t("Pulse on the NMI line (experimental)", "Impuls na linii NMI (eksperymentalnie)")).clicked() {
                tx.send(Cmd::Nmi);
            }
            if ui.checkbox(&mut self.cfg.turbo, "Turbo").on_hover_text(t("No synchronisation with the wall clock, fast FDC (F9)", "Bez synchronizacji z zegarem, szybkie FDC (F9)")).changed() {
                tx.send(Cmd::Turbo(self.cfg.turbo));
                if let Some(t) = &self.term {
                    t.send(TCmd::Turbo(self.cfg.turbo));
                }
            }
        });
        let st = &self.st;
        let up = st.uptime as u64;
        ui.horizontal_wrapped(|ui| {
            ui.monospace(tf!(
                "{:6.2} MHz ({:4.0}%)  time {:02}:{:02}:{:02}  PC={:04X}  latch {:02X} (bank {}, {})  latch2 {:02X}  DIP {:02X}",
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
                ui.colored_label(WARN, t("paused", "wstrzymany"));
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
            bit(ui, &mut self.cfg, 6, t("bit 6 (1 = no drive C:)", "bit 6 (1 = brak napędu C:)"));
            bit(ui, &mut self.cfg, 1, "bit 1 (?)");
        });
        let d = self.cfg.dip;
        ui.horizontal_wrapped(|ui| {
            let c = if d & 0x40 != 0 { t("none", "brak").to_string() } else { format!("{} KB", disk::format_for(d, 1).kbytes()) };
            ui.label(tf!("DIP {:02X}: floppies B: {} KB, C: {}", "DIP {:02X}: dyskietki B: {} KB, C: {}", d, disk::format_for(d, 0).kbytes(), c));
            if d & bosman8::p98::DIP_MASK != self.st.dip & bosman8::p98::DIP_MASK && self.emu.is_some() {
                ui.colored_label(WARN, t("— takes effect after “Power cycle”", "— zadziała po „Wyłącz/włącz”"));
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
                    ui.weak(t("(empty)", "(pusty)"));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Insert…", "Włóż…")).clicked() {
                if let Some(p) = image_dialog(self.cfg.drives[d].path.as_deref()).pick_file() {
                    tx.send(Cmd::Insert { drive: d, path: p.clone(), write_protect: self.cfg.drives[d].write_protect });
                    self.cfg.drives[d].path = Some(p);
                }
            }
            if ui.add_enabled(ds.inserted, egui::Button::new(t("Eject", "Wysuń"))).clicked() {
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
            if ui
                .button(t("New…", "Nowa…"))
                .on_hover_text(t("Create a blank formatted floppy and insert it", "Utwórz pustą sformatowaną dyskietkę i włóż ją"))
                .clicked()
            {
                let f = match self.new_kb[d] {
                    800 => disk::F800,
                    400 => disk::F400,
                    _ => disk::F200,
                };
                let dlg = image_dialog(self.cfg.drives[d].path.as_deref())
                    .set_file_name(format!("{}{}.img", t("disk", "dysk"), DRIVE_NAMES[d].trim_end_matches(':').to_lowercase()));
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
            if ui.checkbox(&mut wp, t("Write-protected", "Zabezpieczona przed zapisem")).changed() {
                self.cfg.drives[d].write_protect = wp;
                tx.send(Cmd::WriteProtect(d, wp));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(tf!("format per DIP: {} KB ({} B)", "format wg DIP: {} KB ({} B)", expected.kbytes(), expected.image_len()));
            if ds.inserted && ds.image_len != expected.image_len() {
                ui.colored_label(WARN, tf!("⚠ the image has {} B", "⚠ obraz ma {} B", ds.image_len));
            }
            if d == 1 && dip & 0x40 != 0 {
                ui.colored_label(WARN, t("⚠ DIP bit 6 = 1: the ROM does not use drive C:", "⚠ DIP bit 6 = 1: ROM nie używa napędu C:"));
            }
        });
    }

    fn link_panel(&mut self, ui: &mut Ui, slot: Slot) {
        let ls: LinkStatus = match slot {
            Slot::SioA => self.st.term.clone(),
            Slot::SioB => self.st.aux.clone(),
            Slot::Term => match &self.term {
                Some(t) => {
                    let f = t.frame();
                    LinkStatus { describe: f.link.clone(), connected: f.link_connected, error: None }
                }
                None => LinkStatus::default(),
            },
        };
        let ports = self.serial_ports.clone();
        let mut kinds = vec![LinkKind::None, LinkKind::TcpListen, LinkKind::TcpConnect, LinkKind::Serial, LinkKind::Spec];
        if slot == Slot::SioA && self.mode == Mode::Full {
            kinds.insert(0, LinkKind::Terminal);
        }
        let cfg: &mut LinkCfg = match slot {
            Slot::SioA => &mut self.cfg.term,
            Slot::SioB => &mut self.cfg.aux,
            Slot::Term => &mut self.cfg.terminal.link,
        };
        let mut apply = false;
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("linkkind", slot as u8))
                .selected_text(kind_name(cfg.kind))
                .show_ui(ui, |ui| {
                    for k in kinds {
                        ui.selectable_value(&mut cfg.kind, k, kind_name(k));
                    }
                });
            match cfg.kind {
                LinkKind::Terminal => {}
                LinkKind::None => {}
                LinkKind::TcpListen => {
                    ui.label(t("address:port", "adres:port"));
                    ui.add(egui::TextEdit::singleline(&mut cfg.listen).desired_width(150.0));
                }
                LinkKind::TcpConnect => {
                    ui.label("host:port");
                    ui.add(egui::TextEdit::singleline(&mut cfg.connect).desired_width(150.0));
                }
                LinkKind::Serial => {
                    egui::ComboBox::from_id_salt(("ports", slot as u8))
                        .selected_text(if cfg.serial_dev.is_empty() { "(port)" } else { cfg.serial_dev.as_str() })
                        .show_ui(ui, |ui| {
                            if ports.is_empty() {
                                ui.weak(t("no ports", "brak portów"));
                            }
                            for p in &ports {
                                ui.selectable_value(&mut cfg.serial_dev, p.clone(), p);
                            }
                        });
                    ui.add(egui::TextEdit::singleline(&mut cfg.serial_dev).desired_width(120.0).hint_text("/dev/ttyUSB0"));
                    if ui.small_button("⟳").on_hover_text(t("Refresh the port list", "Odśwież listę portów")).clicked() {
                        refresh = true;
                    }
                    egui::ComboBox::from_id_salt(("baud", slot as u8))
                        .width(80.0)
                        .selected_text(format!("{} bd", cfg.baud))
                        .show_ui(ui, |ui| {
                            for b in BAUDS {
                                ui.selectable_value(&mut cfg.baud, b, b.to_string());
                            }
                        });
                    egui::ComboBox::from_id_salt(("format", slot as u8))
                        .width(50.0)
                        .selected_text(cfg.format.clone())
                        .show_ui(ui, |ui| {
                            for f in ["8N1", "8N2", "8E1", "8O1", "7E1", "7O1"] {
                                ui.selectable_value(&mut cfg.format, f.to_string(), f);
                            }
                        })
                        .response
                        .on_hover_text(t(
                            "Character format when the port opens; without --fixed-baud the ROM sets it anyway \
                             (terminal line: 8N2)",
                            "Format znaku przy otwarciu portu; bez --fixed-baud ROM i tak go przestawia \
                             (łącze z terminalem: 8N2)",
                        ));
                    ui.checkbox(&mut cfg.rtscts, "RTS/CTS").on_hover_text(t(
                        "Hardware flow control with the port's RTS/CTS lines: the other device \
                         holds the transmission (terminal → SIO-A with Auto Enables). Only with a cable \
                         that has the CTS line — without it the emulator would wait forever.",
                        "Sprzętowe sterowanie przepływem liniami RTS/CTS portu: drugie urządzenie \
                         wstrzymuje nadawanie (terminal → SIO-A z Auto Enables). Tylko z kablem, \
                         który ma linię CTS — bez niej emulator czekałby w nieskończoność.",
                    ));
                }
                LinkKind::Spec => {
                    ui.add(egui::TextEdit::singleline(&mut cfg.spec).desired_width(200.0))
                        .on_hover_text("none | loopback | stdio | tcp-listen:ADDR:PORT | tcp:HOST:PORT[,raw] | serial:DEV@BAUD[,8N2][,rtscts]");
                }
            }
            if ui.button(t("Apply", "Zastosuj")).on_hover_text(cfg.spec()).clicked() {
                apply = true;
            }
        });
        if apply {
            let spec = cfg.spec();
            self.apply_link(slot, spec);
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
                        if ui.button(t("File…", "Plik…")).clicked() {
                            let mut dlg = rfd::FileDialog::new().add_filter(t("Text", "Tekst"), &["txt", "prn"]);
                            if let Some(f) = &p.file {
                                if let Some(dir) = f.parent() {
                                    dlg = dlg.set_directory(dir);
                                }
                                if let Some(n) = f.file_name() {
                                    dlg = dlg.set_file_name(n.to_string_lossy());
                                }
                            } else {
                                dlg = dlg.set_file_name(t("printer.txt", "drukarka.txt"));
                            }
                            if let Some(f) = dlg.save_file() {
                                p.file = Some(f);
                            }
                        }
                        ui.label(p.file.as_ref().map_or(t("(not chosen)", "(nie wybrano)").into(), |f| f.display().to_string()));
                        ui.radio_value(&mut p.append, true, t("append", "dopisuj"));
                        ui.radio_value(&mut p.append, false, t("overwrite", "nadpisuj"));
                    }
                    PrinterKind::Link => {
                        ui.add(egui::TextEdit::singleline(&mut p.link).desired_width(220.0))
                            .on_hover_text("tcp:HOST:PORT[,raw] | tcp-listen:ADDR:PORT | serial:DEV@BAUD[,8N2][,rtscts]");
                    }
                    PrinterKind::Device => {
                        ui.add(egui::TextEdit::singleline(&mut p.device).desired_width(160.0))
                            .on_hover_text("/dev/usb/lp0, /dev/lp0, LPT1 …");
                    }
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let label = if open { t("Close", "Zamknij") } else { t("Open", "Otwórz") };
            if ui.add_enabled(open || p.kind != PrinterKind::None, egui::Button::new(label)).clicked() {
                toggle = true;
            }
            led(ui, open, GREEN);
            match (&self.st.printer, &self.st.printer_error) {
                (Some(d), _) => ui.weak(tf!("open: {d}", "otwarte: {d}")),
                (None, Some(e)) => ui.colored_label(RED, e),
                (None, None) => ui.weak(t("closed (bytes are discarded)", "zamknięte (bajty są odrzucane)")),
            };
            ui.label(tf!("printed: {} B", "wydrukowano: {} B", self.st.printed));
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

    /// Connect a line to a new spec.
    fn apply_link(&mut self, slot: Slot, spec: String) {
        match slot {
            Slot::Term => {
                let link = self.terminal_link();
                if let Some(t) = &self.term {
                    t.send(TCmd::Link(link));
                }
            }
            Slot::SioB => self.send(Cmd::Link { aux: true, spec }),
            Slot::SioA => {
                self.send(Cmd::Link { aux: false, spec });
                // The built-in terminal follows: connected again, or left without a line.
                if self.term.is_some() {
                    let link = self.terminal_link();
                    if let Some(t) = &self.term {
                        t.send(TCmd::Link(link));
                    }
                }
            }
        }
    }

    fn terminal_panel(&mut self, ui: &mut Ui) {
        let mut restart = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("ROM:");
            let desc = if self.term_rom_desc.is_empty() { t("built-in (90.09.11)", "wbudowany (90.09.11)") } else { &self.term_rom_desc };
            ui.label(RichText::new(desc).monospace()).on_hover_text(t("ROM of the ANG-3001 terminal", "ROM terminala ANG-3001"));
            if ui.button(t("Choose…", "Wybierz…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("ROM", &["bin", "BIN", "rom"]).pick_file() {
                    self.cfg.terminal.rom = Some(p);
                    restart = true;
                }
            }
            if self.cfg.terminal.rom.is_some() && ui.button(t("Built-in", "Wbudowany")).on_hover_text(t("ROM 90.09.11 built into the program", "ROM 90.09.11 wbudowany w program")).clicked() {
                self.cfg.terminal.rom = None;
                restart = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Colour:", "Kolor:"));
            let before = self.cfg.terminal.color.clone();
            for (c, name) in [("green", t("green", "zielony")), ("amber", t("amber", "bursztynowy")), ("white", t("white", "biały"))] {
                ui.radio_value(&mut self.cfg.terminal.color, c.to_string(), name);
            }
            if self.cfg.terminal.color != before {
                if let Some(t) = &self.term {
                    t.send(TCmd::Colors(self.cfg.terminal.fg(), 0));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(t("First picture line:", "Pierwsza linia obrazu:"));
            let before = self.cfg.terminal.line_offset;
            ui.radio_value(&mut self.cfg.terminal.line_offset, 0, t("0 (message band at the top)", "0 (pas komunikatów u góry)"));
            ui.radio_value(&mut self.cfg.terminal.line_offset, 16, t("16 (row 0 at the top)", "16 (wiersz 0 u góry)"));
            if self.cfg.terminal.line_offset != before {
                if let Some(t) = &self.term {
                    t.send(TCmd::LineOffset(self.cfg.terminal.line_offset));
                }
            }
        });
        if ui
            .button(t("Terminal cold reset", "Reset terminala (zimny)"))
            .on_hover_text(t("Start the terminal emulator again (also after changing the ROM)", "Uruchom emulator terminala od nowa (także po zmianie ROM-u)"))
            .clicked()
        {
            restart = true;
        }
        if restart {
            self.start_terminal();
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

        // Keyboard → terminal, unless a text field is being edited. Buttons
        // must not keep the focus, or Enter / Space would press them again.
        let mut actions = Vec::new();
        if let Some(t) = &self.term {
            if !ctx.wants_keyboard_input() {
                if let Some(id) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(id));
                }
            }
            actions.extend(self.view.keyboard(ctx, t));
        }

        let fg = {
            let c = self.cfg.terminal.fg();
            Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
        };
        egui::TopBottomPanel::bottom("bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(t) = &self.term {
                    actions.extend(self.view.bar(ui, t, fg));
                }
                if self.mode.has_computer() {
                    ui.separator();
                    for d in 0..2 {
                        let ds = &self.st.drives[d];
                        let name = ds.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                        lamp(ui, ds.active, GREEN, DRIVE_NAMES[d]).on_hover_text(name.unwrap_or_else(|| t("(empty)", "(pusta)").into()));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let panel = if self.cfg.panel_open { "⚙ ▶" } else { "⚙ ◀" };
                    if ui.selectable_label(self.cfg.panel_open, panel).on_hover_text(t("Show / hide the settings", "Pokaż / schowaj ustawienia")).clicked() {
                        self.cfg.panel_open = !self.cfg.panel_open;
                    }
                    if ui
                        .add(egui::Button::new("TURBO").shortcut_text("F9").selected(self.cfg.turbo))
                        .on_hover_text(t("No synchronisation with the wall clock", "Bez synchronizacji z zegarem"))
                        .clicked()
                    {
                        actions.push(FAction::Turbo);
                    }
                    if self.mode.has_computer() {
                        if ui
                            .button(t("Power cycle", "Wyłącz/włącz"))
                            .on_hover_text(t("Cold start of the computer (DIP jumpers applied, banner)", "Zimny start komputera (zworki DIP zastosowane, baner)"))
                            .clicked()
                        {
                            self.send(Cmd::ColdReset(self.cfg.dip));
                        }
                        if ui
                            .button(t("Computer RESET", "RESET komputera"))
                            .on_hover_text(t("The Bosman's RESET button: CPU only", "Przycisk RESET Bosmana: tylko procesor"))
                            .clicked()
                        {
                            self.send(Cmd::WarmReset);
                        }
                    }
                });
            });
        });

        if self.cfg.panel_open {
            egui::SidePanel::right("settings").resizable(true).default_width(400.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ctx, |ui| {
            if self.screen_shown() {
                if let Some(t) = &self.term {
                    self.view.screen(ui, t);
                }
            } else {
                egui::Frame::NONE.inner_margin(12.0).show(ui, |ui| self.no_screen(ui));
            }
        });

        for a in actions {
            self.apply(a);
        }
        if self.view.arrows != self.cfg.terminal.arrows() {
            self.cfg.terminal.set_arrows(self.view.arrows);
        }
        self.save_config(false);
        ctx.request_repaint_after(Duration::from_millis(if self.term.is_some() { 16 } else { 100 }));
    }
}

impl App {
    /// Contents of the side panel, per mode.
    fn settings(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Language / Język:");
            let cur = i18n::get();
            egui::ComboBox::from_id_salt("lang").selected_text(cur.name()).show_ui(ui, |ui| {
                for l in Lang::ALL {
                    if ui.selectable_label(cur == l, l.name()).clicked() {
                        i18n::set(l);
                        self.cfg.lang = l.code().into();
                    }
                }
            });
        });
        if let Some(e) = self.rom_error.clone() {
            ui.colored_label(RED, tf!("No computer ROM: {e}", "Brak ROM-u komputera: {e}"));
            if ui.button(t("Choose a ROM file…", "Wybierz plik ROM…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("ROM", &["bin", "rom"]).pick_file() {
                    self.cfg.rom = Some(p);
                    self.start_computer();
                }
            }
            ui.separator();
        }
        if self.mode.has_computer() {
            section(ui, t("Computer", "Komputer"), |ui| self.machine_panel(ui));
            section(ui, t("Floppy drives", "Stacje dyskietek"), |ui| {
                self.drive_panel(ui, 0);
                ui.separator();
                self.drive_panel(ui, 1);
            });
            section(ui, t("Port 0x98 jumpers (DIP)", "Zworki portu 0x98 (DIP)"), |ui| self.dip_panel(ui));
            section(ui, "SIO-A — terminal", |ui| self.link_panel(ui, Slot::SioA));
            section(ui, "SIO-B — V.24", |ui| self.link_panel(ui, Slot::SioB));
            section(ui, t("Parallel port (Centronics)", "Port równoległy (Centronics)"), |ui| self.printer_panel(ui));
        }
        if self.mode == Mode::Terminal {
            section(ui, t("Terminal line", "Łącze terminala"), |ui| self.link_panel(ui, Slot::Term));
        }
        if self.mode.has_terminal() {
            section(ui, t("ANG-3001 terminal", "Terminal ANG-3001"), |ui| self.terminal_panel(ui));
        }
        if self.mode.has_computer() {
            let r = egui::CollapsingHeader::new(t("SIO-A output monitor", "Podgląd wyjścia SIO-A"))
                .id_salt("monitor")
                .default_open(self.cfg.show_monitor)
                .show(ui, |ui| self.monitor_panel(ui));
            self.cfg.show_monitor = r.body_returned.is_some();
            if let Some(p) = &self.rom_path {
                ui.weak(tf!("Computer ROM: {}", "ROM komputera: {}", p.display()));
            }
        }
        egui::CollapsingHeader::new(tf!("Messages ({})", "Komunikaty ({})", self.messages.len()))
            .id_salt("messages")
            .default_open(false)
            .show(ui, |ui| {
            if ui.small_button(t("clear", "wyczyść")).clicked() {
                self.messages.clear();
            }
            for m in &self.messages {
                ui.label(RichText::new(m).small());
            }
        });
    }

    /// Centre of the window when the terminal is not on screen.
    fn no_screen(&mut self, ui: &mut Ui) {
        let ls = self.st.term.clone();
        ui.label(RichText::new("SIO-A (terminal)").strong().color(Color32::LIGHT_GRAY));
        ui.horizontal(|ui| {
            led(ui, ls.connected && ls.error.is_none(), GREEN);
            match &ls.error {
                Some(e) => ui.colored_label(RED, e),
                None => ui.label(RichText::new(&ls.describe).color(Color32::LIGHT_GRAY)),
            };
        });
        if self.mode == Mode::Full {
            ui.label(
                RichText::new(t(
                    "The built-in terminal is disconnected — SIO-A is set to another line (panel ⚙ → SIO-A).",
                    "Wbudowany terminal jest odłączony — SIO-A ustawione na inne łącze (panel ⚙ → SIO-A).",
                ))
                    .color(Color32::GRAY),
            );
        } else {
            ui.label(
                RichText::new(t(
                    "Connect a terminal: a second window “bosman8 terminal” (TCP 127.0.0.1:3001) or a real ANG-3001 on a serial port.",
                    "Podłącz terminal: drugie okno „bosman8 terminal” (TCP 127.0.0.1:3001) albo prawdziwy ANG-3001 przez port szeregowy.",
                ))
                    .color(Color32::GRAY),
            );
        }
        ui.add_space(8.0);
        ui.label(RichText::new(t("SIO-A output", "Wyjście SIO-A")).strong().color(Color32::LIGHT_GRAY));
        let text = monitor_text(self.st.monitor.iter().copied());
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).monospace().size(12.0).color(Color32::from_rgb(0x33, 0xFF, 0x66))).wrap());
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.term = None;
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
    let mut dlg = rfd::FileDialog::new().add_filter(t("Floppy image", "Obraz dyskietki"), &["img", "dsk", "raw", "hfe"]).add_filter(t("All", "Wszystkie"), &["*"]);
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
        LinkKind::None => t("none", "brak"),
        LinkKind::TcpListen => t("TCP — listen", "TCP — nasłuch"),
        LinkKind::TcpConnect => t("TCP — connect", "TCP — połączenie"),
        LinkKind::Serial => t("host serial port", "port szeregowy hosta"),
        LinkKind::Spec => t("other spec", "inna specyfikacja"),
        LinkKind::Terminal => t("built-in ANG-3001 terminal", "wbudowany terminal ANG-3001"),
    }
}

fn printer_name(k: PrinterKind) -> &'static str {
    match k {
        PrinterKind::None => t("none", "brak"),
        PrinterKind::File => t("file", "plik"),
        PrinterKind::Link => t("link (TCP / serial port)", "łącze (TCP / port szeregowy)"),
        PrinterKind::Device => t("host device", "urządzenie hosta"),
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
