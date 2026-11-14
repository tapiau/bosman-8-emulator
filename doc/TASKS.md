# Zadania

Legenda: ✅ zrobione · 🔄 w trakcie · ⏳ zaplanowane · ❓ wymaga danych/decyzji

## Faza 0 — rozpoznanie
- ✅ Przegląd dokumentacji `bosman-8-rom` i `bosman-8-ang-3000-rom`
- ✅ Ustalenie mapy pamięci i portów Bosmana z kodu ROM (zob. [bosman8-hardware.md](bosman8-hardware.md))
- ✅ Ustalenie wideo/klawiatury/UART terminala (zob. [ang3001-hardware.md](ang3001-hardware.md))
- ✅ Wybór technologii: Rust + iz80 (zob. [ARCHITECTURE.md](ARCHITECTURE.md))

## Faza 1 — szkielet
- ✅ T1.1 Workspace Cargo, crate'y `b8-link`, `b8-chips`, `bosman8`, `ang3001`
- ✅ T1.2 `b8-link`: trait endpointu + backendy null / TCP listen / TCP connect / stdio / serial hosta
- ✅ T1.3 `b8-chips`: 74LS259, 8253, Z80-SIO (tryb bajtowy), 8251

## Faza 2 — Bosman-8
- ✅ T2.1 Pamięć: ROM 32 KB / 8 banków RAM 64 KB przełączających 0000–EFFF, wspólne F000–FFFF (opcjonalnie ROM 64 KB, Q0 = górna połowa)
- ✅ T2.2 Porty: zatrzask 0x00–0x0F, SIO 0x80–0x83, 8253 0x84–0x87, DIP 0x98, drugi zatrzask 0xF0–0xFF, NMI = INTRQ·Q1
- ✅ T2.3 Binarka CLI: `--term tcp-listen:…|stdio|serial:…`, `--aux …` (SIO-B), `--trace-io`, `--trace-cpu-from`, `--turbo`
- ✅ T2.4 Boot do znaku zachęty CCP z RAM-dyskiem A: (test `crates/bosman8/tests/boot.rs`)
- ✅ T2.5 WD1770 + obrazy dyskietek (B:, C:): 5×1024 B/ścieżkę, 200/400/800 KB wg zworek 0x98 (zob. bosman8-hardware.md)
- ✅ T2.6 Centronics (port 0x98 OUT) → plik (`--printer`)
- ✅ T2.7 GUI Bosmana (eframe/egui, domyślny tryb `bosman8`, `--no-gui` = konsola): RESET/zimny start, stacje B:/C: z wyborem obrazu, zworki DIP, SIO-A/SIO-B i Centronics konfigurowane w locie, ustawienia w JSON

## Faza 3 — ANG-3001
- ✅ T3.1 Pamięć: ROM 16 KB + RAM (też pod ROM-em, porty 0x0B/0x0A), wideo 512×256 z 0x8000, scroll port 0x80, `--line-offset` (domyślnie 0)
- ✅ T3.2 8251 na 0x90/0x91, timer 0x88–0x8B (dźwięk: `beep()` w rdzeniu, w GUI tylko tytuł okna)
- ✅ T3.3 Klawiatura: porty 0x8C/0x8D/0x8E, VSYNC na bicie 2 portu 0x8E
- ✅ T3.4 Okno (minifb), mapowanie klawiszy hosta, BREAK, LOCAL; tryb `--headless` ze zrzutem PGM
- ✅ T3.5 Weryfikacja ekranu startowego (baner „ANG-3000”) i offsetu wierszy ❓ offset do potwierdzenia zdjęciem prawdziwego ekranu
- ✅ T3.7 Strzałki w terminalu: domyślnie cyfry 8/2/4/6 (gry, np. SOKOBAN); lista ARROWS na pasku, F6, `--arrows numpad|vt52|wordstar|vim|none` (`off`)
- ✅ T3.6 Pasek przycisków w oknie terminala (RESET, BREAK, LOCAL, STOP, CAPS, TURBO + kontrolki LINK/BEEP/WAIT), API `set_stop`, `set_local`, `mode()`

## Faza 4 — integracja
- ✅ T4.1 Bosman ↔ ANG po TCP: znak zachęty `A>` na ekranie terminala
- ⏳ T4.2 Uruchomienie demek z `bosman-8-ang-3000-rom/demo` (wstrzyknięcie plików na RAM-dysk)
- ✅ T4.3 Narzędzie `b8disk` (new/ls/put/get/rm) dla obrazów dyskietek 200/400/800 KB; obraz `disks/narzedzia.img`
- ✅ T4.6 Obrazy Gotka `.hfe` (HxC v1) bezpośrednio w `--disk-b/c`, GUI i `b8disk`; zapis w miejscu (tylko pola danych zmienionych sektorów)
- ✅ T4.8 Sterowanie przepływem RTS/CTS: SIO-A z Auto Enables, terminal wyłącza RTS przy zaległości; TCP między emulatorami przez podzbiór RFC 2217 (`,raw` wyłącza); `serial:…,rtscts` — linie RTS/CTS portu hosta (Bosman ↔ prawdziwy terminal, ANG ↔ prawdziwy Bosman)
- ✅ T4.7 ROM terminala ANG-3001 90.09.11 (`--rom roms/ang3001/ANG-3001_90.09.11.BIN`): profile ROM-ów, DSR 8251 nieaktywny; gry z dyskietki GRY (SOKOBAN) rysują grafikę

- ✅ T4.11 Język interfejsu: domyślnie angielski, polski przez `--lang pl`, `"lang"` w ustawieniach albo wybór w panelu (`gui/i18n.rs`); skróty `.desktop` z wariantami `[pl]`
- ✅ T4.10 Ikona: koło sterowe z 8 szprychami i mosiężną piastą z „8” (`packaging/icons/`, źródła SVG + `render.sh`): ikona okna, ikony `hicolor` i `Icon=` w `.deb`, ikona `.exe` w Windows (`build.rs` + `winresource`)
- ✅ T4.9 Jedna binarka `bosman8` z trybami: **full** (komputer + terminal w jednym oknie egui, łącze w pamięci z RTS/CTS), **computer** (sam Bosman, SIO-A na TCP/serial), **terminal** (sam ANG-3001, np. do prawdziwego Bosmana); `ang3001` jako dowiązanie (argv[0]) = tryb terminal; konfiguracja per tryb; terminal przeniesiony z minifb na egui (zob. dziennik 2026-11-14) ❓ klawiatura w oknie sprawdzona testem bez okna — do ręcznego sprawdzenia przez użytkownika

## Faza 4b — łącze między Bosmanami
- ✅ T4.4 Napędy D/E/F przez SIO-B (master ↔ slave `DEBUG`): D/E/F = A/B/C serwera, `DIR`, odczyt (`E:PIP A:=E:STAT.COM`) i zapis działają między dwoma emulatorami (TCP; test `crates/bosman8/tests/interlink.rs`); `--trace-aux`. ❓ rzeczywiste ROM-y UNIMOR ZJ nie mają rozkazu DEBUG 60 (serwer D/E/F); podłączony egzemplarz zidentyfikowany jako 1987-10-20; BSRV.COM dla obu ROM-ów ZJ i instalator przez DEBUG (tools/bsrv); ✅ sprzęt: katalog E:, odczyt/zapis pliku z porównaniem, kasowanie; wymagane 20 ms ochrony TX przed zmianą prędkości USB — zob. [bosman8-interlink.md](bosman8-interlink.md)

- ✅ T4.5 Port szeregowy hosta podąża za prędkością/formatem programowanym w SIO/8251 (`set_line_params`, `--fixed-baud`)

## Faza 5 — później
- ⏳ Dźwięk (cpal) w terminalu
- ⏳ Wersja webowa (WASM + canvas, łącze przez WebSocket albo oba emulatory w jednej stronie)
- ⏳ Debugger (breakpointy, podgląd pamięci)
- ⏳ ANG-3001: przełączanie stron obrazu przez zatrzask Q4 (porty 0x08/0x09, oba ROM-y) i Q6 (0x0C/0x0D, tylko 90.09.11) — dziś ignorowane; hipoteza: druga strona w 0xC000
- ⏳ Bosman `--term stdio`: polecenie wpisane w trakcie pracy programu ginie po ciepłym starcie (bez `--fixed-baud` całe wejście po pierwszym ciepłym starcie) — było tak też przed 0.4.1
- ❓ ANG-3001: czy DSR 8251 jest na płycie do czegoś podłączony (ROM 90.09.11 używa go jako sygnału „uwagi”)
- ❓ Czy domyślnym ROM-em terminala ma zostać ZJ wersja PG, czy 90.09.11

## Dziennik
(uzupełniany na bieżąco — najnowsze na dole)
- 2026-11-13: rozpoznanie sprzętu z kodu ROM; dokumentacja w `doc/`; szkielet workspace z interfejsami
  (`b8-link`, `b8-chips`). T1.2 i T1.3 zlecone równolegle subagentom; T2/T3 ruszą po nich.
- 2026-11-13: T1.2 (`b8-link`, 6 testów) i T1.3 (`b8-chips`, 58 testów) gotowe. Druga fala: T2.x (Bosman) i T3.x (ANG) równolegle u subagentów.
- 2026-11-13: T2.1–T2.6 gotowe (`crates/bosman8`: lib + CLI). ROM bootuje do `A>` bez łatek; DIR/SAVE na RAM-dysku
  i dyskietkach B:/C:, `^P` → drukarka. Ustalone: NMI = WD1770 INTRQ bramkowane Q1 zatrzasku 0xF0–0xFF,
  port 0x98 bit0 = /DRQ, bit2 = gotowość napędu, bit7 = drukarka gotowa; format dyskietek z tablic DPB.
- 2026-11-13: T3.1–T3.5 gotowe (`crates/ang3001`: lib + GUI minifb/headless, 24 testy). Baner „ANG-3000 V.01 F0CD0D00871127”
  od razu bez łatek; demka .TER (FIRE, SNAKE, CUBE, SIERP…) działają przez `--load-ter`. Ustalenia: RAM pod ROM-em
  (`OUT 0Bh` = ROM off, `0Ah` = on — używane przez okna `ESC (`/`ESC )`), `ESC =` to wiersz-potem-kolumna (wiki ma
  odwrotnie), polskie litery w foncie pod kodami 0x01–0x0C, bit 4 portu 0x8D = stronicowanie „STOP”. Szczegóły w
  [ang3001-hardware.md](ang3001-hardware.md).
- 2026-11-13: T4.1 — oba emulatory jako osobne procesy po TCP: baner „BOSMAN 8 / CPM/R” w ramce, `DIR` wpisane na
  klawiaturze terminala → „Brak pliku”. Dodane README z instrukcją uruchomienia.
- 2026-11-13: T4.3 — `b8disk` + moduł `cpmfs`; obraz `disks/narzedzia.img` (14 narzędzi CP/M), zweryfikowany
  przez cpmtools i w emulatorze (`DIR B:`, `B:STAT`, `PIP A:=B:...`).
- 2026-11-13: znaleziona w ROM strona serwera łącza D/E/F — tryb slave (0x1564) wchodzi poleceniem CCP `DEBUG`;
  pierwszy test dwóch emulatorów: handshake OK, katalog zdalny jeszcze nie.
- 2026-11-13: T3.6 — pasek przycisków pod ekranem ANG-3001 (rysowany fontem z ROM-u, mysz przez minifb), skróty F7 = LOCAL,
  F8/Scroll Lock = STOP. Z ROM-u: LOCAL to osobna pętla 0x0578 bez flagi w RAM-ie, wyjście = jedno naciśnięcie BREAK;
  STOP (bit 4 portu 0x8D) to stan poziomu, nie zbocze → przełącznik/klawisz z zatrzaskiem. Rdzeń śledzi tryb po PC
  (`TermMode`), `set_local` sam wykonuje sekwencję BREAK/ESC. Testy `crates/ang3001/tests/modes.rs`.
- 2026-11-13: T2.7 — GUI w `crates/bosman8/src/gui/` (feature `gui`, domyślnie włączona; eframe 0.32 + rfd 0.15,
  Rust 1.85 — workspace `resolver = "3"`). Rdzeń: `src/control.rs` (`warm_reset`, `cold_reset`, `set_dip`, `eject_disk`,
  `set_write_protect`, `drive_active`, `set_term_endpoint`/`set_aux_endpoint`, `bank`, `nmi`). Ustalenie: przy `A>`
  CCP działa z ROM-u (zatrzask 0x70, Q7 = 0), więc „reset samego CPU” daje sam znak zachęty tylko, gdy w 0000 jest RAM
  (program z TPA); przy `A>` — pełny start z banerem. Obserwacja ze sprzętu (sam `A>`) nadal niewyjaśniona ❓.
- 2026-11-13: T4.4 — łącze D/E/F działa (DIR, PIP w obie strony, `DIR G:` zwalnia serwer). Protokół rozpisany z kodu:
  rozkazy slave'a 50h–60h (rejestry adresowe FBFA/FBFC/FBFE), serwer dysków po 60h (F0 koniec, F1 nagłówek HOME/
  SETTRK/SETSEC/SELDSK z sumą, F2 odczyt, F3 zapis 128 B), po 60h obie strony przechodzą na 8253 = 6 (20833 bit/s).
  Przyczyna „Brak pliku”: `SIOB_INIT` (0x1487) wyrzuca bajty odebrane w 2 ms po zmianie prędkości, a bajty z TCP
  przychodziły paczkami (`60 00 F1` naraz) → ginął F1. Poprawki: hold-off odbiornika 3 ms po zapisie licznika 8253
  kanału, 3-bajtowe FIFO odbiorcze SIO (`b8-chips`), SIO-B gubi bajty przy wyłączonym odbiorniku. Nowa opcja
  `--trace-aux`.
- 2026-11-13: T4.5 — `Endpoint::set_line_params`; Bosman (SIO-A/B) i ANG (8251) przekazują zaprogramowaną prędkość
  i format, `serial:` przestawia port hosta. Sprawdzone na PL2303HX: 9615 / 20833 / 100000 bit/s dokładnie.
  Odkrycie: łącze Bosman ↔ ANG to async 100000 8N2 (SIO-A WR4=0Ch), nie tryb synchroniczny.
- 2026-11-14: wydania 0.3.2 (runnery macOS 15 / Node 24 — v0.3.0 i v0.3.1 nie zostały opublikowane, ich buildy
  anulowano) i 0.4.0: T4.6 obrazy Gotka `.hfe` (moduł `crates/bosman8/src/hfe.rs`, kontener w `disk.rs`). Odczyt:
  dekodowanie MFM z CRC, pamiętana pozycja pola danych każdego sektora; zapis: przekodowanie tylko pola danych + CRC
  w miejscu, reszta ścieżki bit w bit. Zły format (np. 200 KB przy zworkach 400 KB) → zabezpieczona przed zapisem
  z podpowiedzią. Sprawdzone na `DSKA0001_B8_GRY_COM.hfe` (gry na B:, `SAVE` w CP/M zapisuje się do `.hfe`).
- 2026-11-14: poprawka T2.1 — banki RAM Bosmana mają 64 KB i przełączają 0000–EFFF, wspólne jest tylko F000–FFFF
  (dowody: baner RAM=512 KB i TPA=60 KB, adresowanie RAM-dysku 0x0892 do 0xEFFF). Wcześniej przełączane było tylko
  0000–7FFF, więc `.COM` > ~34 KB nadpisywał zmienne CCP w 0x8800: BLOCK.COM ustawiał bit 0 bajtu 0x8810 (flaga
  `DEBUG`) i CCP wypisywał `Program 0100-A37F` zamiast uruchomić grę; SZACHY psuł znak zachęty (`A5>`). Szczegóły
  w [bosman8-hardware.md](bosman8-hardware.md) (sekcja „Pamięć”).
- 2026-11-14: T4.7 — ROM ANG-3001 90.09.11 wieszał się po banerze: pętla główna testuje `AND 82h` statusu 8251,
  a aktywny DSR (emulator zgłaszał 1 na stałe) to w nim sygnał „uwagi” z pętlą czekania na zgaśnięcie DSR (0x0711).
  DSR jest teraz nieaktywny (`Usart8251::set_dsr`); stary ROM bitu 7 nie czyta. Adresy pętli (koniec startu, BREAK,
  LOCAL, STOP) zależą od ROM-u → `RomProfile` wykrywany po bajtach pętli głównej (stary 04BD/0533/0578/0A23, nowy
  05CF/—/080F/08FB). W 90.09.11 BREAK przy bicie 7 portu 0x8D = 0 wchodzi w LOCAL bez zachęty; automat GUI to
  uwzględnia. Porównanie portów obu ROM-ów: nowy używa nadzbioru (dodatkowo DSR i Q6 zatrzasku 0x0C/0x0D), więc
  stary ROM działa na nowszym sprzęcie bez zmian. SOKOBAN: na 90.09.11 pełna plansza, na starym ciąg liczb (brak
  sekwencji w ROM-ie). Test `crates/ang3001/tests/rom_90_09_11.rs`, opis w [ang3001-hardware.md](ang3001-hardware.md).
- 2026-11-14: ROM-y podzielone na katalogi maszyn: `roms/bosman8/`, `roms/ang3001/` (miejsce na `roms/an2001/`).
  `bosman8` szuka też starej ścieżki `roms/bosman8.bin`; paczka `.deb` zostawia dowiązania pod starymi nazwami, bo
  `/etc/bosman-8-emulator.conf` z wcześniejszych wersji (conffile) może na nie wskazywać. Wydanie 0.4.1.
- 2026-11-14: T4.8 — SOKOBAN reagował na klawisze po dziesiątkach sekund. Pomiar (oba rdzenie w jednym procesie):
  gra w pętli rysuje licznik „Czas” dużymi cyframi, Bosman nadaje ~6400 B/s, terminal (podwajanie pikseli glifu
  0x1BF4) przetwarza ~2200 B/s → zaległość rośnie ~4 KB/s (po 30 s 82 KB ≈ 38 s opóźnienia). Na sprzęcie hamuje to
  Auto Enables SIO-A (WR3 = E1h): bez CTS układ nie nadaje, CONOUT czeka. Dodane: CTS w modelu SIO (`set_cts`),
  `Endpoint::set_rts`/`cts`, RTS terminala z histerezą 32/8 bajtów, w TCP podzbiór RFC 2217 (klient oferuje
  `WILL COM-PORT-OPTION`, serwer przyjmuje tylko na początku połączenia, więc `nc` działa jak dawniej). Wynik przez
  prawdziwe gniazda TCP: w drodze 0–23 bajty (≈10 ms) zamiast 82 KB. Przy okazji: opakowanie `Tap` w GUI Bosmana
  nie przekazywało `write_at`/`set_line_params` (GUI + port szeregowy gubiło odstępy bajtów i zmiany prędkości).
  Testy: `b8-link` (parser, TCP w obie strony, surowy klient, `,raw`), SIO, `bosman8/tests/boot.rs`
  (`terminal_rts_paces_output`), `ang3001/tests/serial.rs` (`rts_off_while_backlog`).
- 2026-11-14: T3.7 — strzałki terminala domyślnie wysyłają cyfry 8/2/4/6 jak klawiatura numeryczna (sterowanie gier).
  Tryb wybiera lista rozwijana ARROWS na pasku (rysowana fontem z ROM-u nad paskiem; klik na pozycji wybiera, klik
  obok zamyka), F6 (następny tryb) i `--arrows numpad|vt52|wordstar|vim|none` (`off` = `none`; vim = k j h l). Przy `--scale 1`
  kontrolki LINK/BEEP/WAIT się nie mieszczą obok listy i są pomijane (stan łącza jest też w tytule okna).
  Przy okazji: `serial:DEV@BAUD,8N2` — format znaku w specyfikacji portu (wcześniej zawsze 8N1, a z `--fixed-baud`
  port zostawał na 1 bicie stopu).
- 2026-11-14: domyślne ROM-y: terminal — 90.09.11 (`ang3001::DEFAULT_ROM` = `ROM_90_09_11`; stary jako `ROM_ZJ_PG`,
  plik przemianowany na oryginalną nazwę `roms/ang3001/ANG3001_ZJ_wersjaPG.BIN`; testy zachowania starego ROM-u
  wskazują go jawnie). Bosman — CPM-R 32 KB, teraz także wbudowany (`bosman8::DEFAULT_ROM`), gdy pliku nie ma obok
  (wcześniej uruchomienie spoza repo kończyło się błędem); obrazy ZJ 16 KB tylko przez `--rom`. `.deb`: dowiązania
  `roms/ang3001.bin` i `roms/ang3001/ang3001.bin` prowadzą do domyślnego 90.09.11.
- 2026-11-14: T4.8 cz. 2 — RTS/CTS na porcie szeregowym: `serial:DEV@BAUD[,FMT],rtscts` (w GUI Bosmana checkbox
  „RTS/CTS” i lista formatu). Wątek `serial-lines` co 1 ms czyta CTS (`read_clear_to_send`) i od razu ustawia RTS
  (`write_request_to_send`); wątek piszący przy wyłączonym CTS wstrzymuje dane. Domyślnie wyłączone (kabel bez CTS
  = wieczne czekanie). Sprawdzone na PL2303 (`/dev/ttyUSB0`): port otwiera się jako `100000 8N2, RTS/CTS`, linia
  CTS od strony kabla aktywna; samo przełączanie RTS widać dopiero po drugiej stronie kabla (port jest otwierany na
  wyłączność) — ❓ do sprawdzenia z prawdziwym Bosmanem/terminalem. Przy okazji: GUI Bosmana błędnie przejmowało
  specyfikację z opcjami po przecinku (`@100000,8N2` → wszystko w nazwie urządzenia).
- 2026-11-14: T4.9 start — jedna binarka zamiast dwóch programów łączonych ręcznie po TCP (prośba: „przeciętnemu
  użytkownikowi odpalanie z palca dwóch aplikacji jest niewygodne”). Ustalone z użytkownikiem: binarka `bosman8`,
  tryby `bosman8 [komplet|komputer|terminal]` (bez trybu = komplet; `full|computer|terminal` też), dwie instancje
  (komputer + terminal) dalej łączą się po TCP jak dawniej; okno: ekran terminala w centrum, zwijany panel boczny z
  ustawieniami, pasek stanu na dole. Architektura: wspólna aplikacja egui z opcjonalnymi komponentami — istniejący
  wątek komputera (`gui/emu.rs`) i nowy wątek terminala (`gui/terminal.rs`, rdzeń `ang3001` bez zmian), w trybie
  komplet połączone `b8_link::pair()` (RTS/CTS już działa). Binarka `ang3001` (minifb) znika: crate zostaje
  biblioteką, tryb terminal bez okna (`--headless`, zrzuty, `--inject`, `--load-ter`) przenoszony do `bosman8`,
  wywołanie jako `ang3001` (dowiązanie) = tryb terminal z dawnymi opcjami.
  - krok 1 ✅: `ArrowMode` + mapowanie znaków (`char_code`: ASCII i polskie litery, `ctrl_code`) przeniesione do
    biblioteki `ang3001` (`crates/ang3001/src/keys.rs`) — wspólne dla nowego GUI i trybu terminal; testy crate'u OK.
  - krok 2 ✅: `crates/bosman8/src/gui/terminal.rs` — wątek terminala (`TermEmu`: rdzeń `ang3001`, takt 20 ms, turbo,
    komendy kanałem, ramka + stan publikowane co klatkę), ekran jako tekstura egui (NEAREST, proporcja 4:3), klawiatura
    z zdarzeń egui (`key_bytes`, `text_bytes`, F6–F12 jak w dawnym oknie), pasek przycisków RESET/BREAK/LOCAL/STOP/CAPS,
    lista ARROWS i lampki LINK/BEEP/WAIT; test mapowania klawiszy.
  - krok 3 ✅: konfiguracja (`gui/config.rs`): `Mode` (komplet/komputer/terminal, nazwy też po angielsku), plik na tryb
    (`komplet.json`, `komputer.json`, `terminal.json`), `LinkKind::Terminal` (SIO-A = wbudowany terminal, spec
    `terminal`), `TermCfg` (ROM, łącze trybu terminal — domyślnie `tcp:127.0.0.1:3001`, kolor, offset, strzałki),
    `panel_open`. Migracja: brak pliku trybu → zarodek ze starego `gui.json` (komplet: SIO-A → wbudowany terminal;
    terminal nie dziedziczy ustawień komputera). Test `modes_and_migration`.
  - krok 4 ✅: okno (`gui/app.rs`) wspólne dla trybów: `App` z opcjonalnym komputerem (`Emu`) i terminalem (`TermEmu`);
    w trybie komplet przy SIO-A = „wbudowany terminal” para `b8_link::pair()` (komputer dostaje swój koniec komendą
    `Cmd::TermEndpoint`, po restarcie komputera para jest zakładana od nowa); panel łączy uogólniony na SIO-A / SIO-B /
    łącze terminala; nowa sekcja „Terminal ANG-3001” (ROM, kolor, pierwsza linia obrazu, zimny reset terminala);
    pasek dolny: przyciski terminala, lampki stacji B:/C:, TURBO (wspólne, F9), RESET / Wyłącz-włącz komputera, ⚙
    (panel); w centrum ekran albo — gdy terminal nie jest podłączony — stan SIO-A i podgląd jego wyjścia. Przyciski
    oddają fokus, gdy klawiatura należy do terminala (inaczej Enter/spacja „klikały” ostatni przycisk).
  - krok 5 ✅: wybór trybu (`main.rs`): `bosman8 [komplet|komputer|terminal]`, wywołanie jako `ang3001` = terminal,
    `bosman8 --no-gui` bez trybu = komputer w wierszu poleceń jak dawniej; opcje i praca bez okna dawnego `ang3001`
    przeniesione do `terminal_cli.rs` (`--headless`, `--screenshot`, `--dump-text`, `--inject`, `--load-ter`, …);
    `--terminal-rom`, `--arrows` dla trybu komplet; okno terminala z `--scale`. Testy: wybór trybu (w tym argv[0]),
    opcje terminala.
  - sprawdzone w oknie (tryb komplet, X11): baner „BOSMAN 8 / CPM/R” i `A>` na ekranie terminala, panel z sekcjami,
    SIO-A „wbudowany terminal ANG-3001” (`pair (A)`, LINK), pasek dolny; tryby bez okna (`terminal --headless`,
    dowiązanie `ang3001`, `komputer --no-gui`) działają.
  - test `gui::tests::full_mode_computer_and_terminal_talk` ✅: wątek komputera + wątek terminala połączone parą jak w
    oknie → baner i `A>` na ekranie terminala, `DIR` wpisane z klawiatury terminala → „Brak pliku” od CCP.
  - krok 6 ✅: binarka `ang3001` (minifb, `main.rs`, `toolbar.rs`) usunięta, crate `ang3001` = sama biblioteka (minifb
    znika z zależności); `bosman8` zależy od niej zawsze (tryb terminal bez okna działa też w buildzie bez GUI).
    Paczki: w archiwach Linux/macOS i w `.deb` dowiązanie `ang3001 → bosman8`, w archiwum Windows `ang3001.cmd`
    (`bosman8 terminal`); menu `.deb`: „Bosman-8” (komplet), „Bosman-8 — komputer”, „ANG-3001 — terminal”.
    Podczas budowania znów zabrakło miejsca na `/home` (632 MB) — wyczyszczony profil debug `target/` (7,2 GB artefaktów).
  - sprawdzone w oknach: `bosman8 komputer` + `bosman8 terminal` naraz (dwie instancje, osobne pliki ustawień) łączą
    się po TCP z RTS/CTS — terminal pokazuje baner i `A>`, okno komputera w centrum stan SIO-A i jego surowe wyjście.
  - dokumentacja: README (tryby, szybki start, okno, ustawienia per tryb, paczki), `ARCHITECTURE.md` (struktura,
    tryb komplet), opisy paczki `.deb` i skróty w menu.
  - T4.9 ✅ całość: testy workspace 145/0, build bez GUI OK, lokalnie zbudowana paczka `.deb` zawiera `bosman8`,
    `b8disk`, dowiązanie `ang3001 → bosman8` i trzy skróty w menu. Nie sprawdzone automatycznie: pisanie z klawiatury
    w samym oknie (brak narzędzia do wstrzykiwania klawiszy w tym środowisku) — ścieżka klawiatura → terminal →
    komputer pokryta testem `full_mode_computer_and_terminal_talk`, mapowanie klawiszy egui testem `keys_map_like_the_old_window`.
- 2026-11-14: T4.9 — nazwy trybów tylko po angielsku: `full`, `computer`, `terminal` (polskie „komplet”/„komputer”
  były roboczym podsumowaniem użytkownika, nie nazwami do `--help` i dokumentacji; parser ich już nie przyjmuje).
  Pliki ustawień `full.json`, `computer.json`, `terminal.json`; tytuły okien i skróty w menu „Bosman-8 (computer)”,
  „ANG-3001 (terminal)”. Starsze wpisy dziennika powyżej używają roboczych nazw.
- 2026-11-14: T4.10 — ikona programu: drewniane koło sterowe (8 szprych z uchwytami — jak „8” w nazwie) z mosiężną
  piastą i cyfrą „8” rysowaną ścieżką (bez zależności od czcionek). Trzy warianty SVG: pełny (≥ 48 px), uproszczony
  (24–32 px), minimalny (16 px — sama obręcz i piasta, inaczej „8” zlewała się w plamę); `render.sh` robi z nich PNG,
  `bosman8.ico` i surowe RGBA 128×128 wbudowane w program. Okno: `with_icon` + `app_id` „bosman8” (sprawdzone:
  `_NET_WM_ICON` 128×128, `WM_CLASS` „bosman8” = `StartupWMClass` skrótów). `.deb`: ikony w
  `/usr/share/icons/hicolor/{16..256}/apps` i `scalable/apps/bosman8.svg`, `Icon=bosman8` w trzech skrótach.
  Windows: `crates/bosman8/build.rs` osadza `bosman8.ico` w plikach `.exe` (crate `winresource`, tylko przy
  kompilacji na Windows) — ❓ sprawdzi dopiero build w CI na Windows.
- 2026-11-14: wydanie 0.5.0 — jedna binarka z trybami `full` / `computer` / `terminal` (T4.9) i ikona (T4.10);
  testy workspace 145/0 przed wydaniem.
- 2026-11-14: wydanie 0.5.0 opublikowane i sprawdzone: buildy i CI na 3 systemach OK; `.deb` z `bosman8`, `b8disk`,
  dowiązaniem `ang3001`, trzema skrótami i ikonami `hicolor`; archiwum Windows z `ang3001.cmd`; ikona osadzona w
  `bosman8.exe` (zasoby `.rsrc` z obrazami ikony).
- 2026-11-14: T4.11 — język interfejsu. Dotąd okno było po polsku; teraz domyślnie po angielsku, polski do wyboru:
  `--lang pl` (tryby full / computer / terminal; `bosman8 computer --no-gui` przyjmuje i ignoruje), pole `lang` w pliku
  ustawień, lista „Language / Język” na górze panelu (przełącza od razu). Implementacja: `gui/i18n.rs` — bieżący język
  w zmiennej atomowej, napisy jako pary w miejscu użycia: `t("English", "Polski")`, makro `tf!` dla `format!`
  (przy dwóch językach czytelniej niż pliki z kluczami). Przetłumaczone wszystkie napisy okna (`app.rs`,
  `terminal.rs`); identyfikatory zwijanych sekcji niezależne od języka (`id_salt`), opis wbudowanego ROM-u terminala
  składany przy rysowaniu. Komunikaty wątku komputera i CLI były już po angielsku. Skróty `.desktop`: angielskie
  `GenericName`/`Comment` + warianty `[pl]`. Sprawdzone zrzutami panelu w obu językach; test `i18n::parse_and_switch`.
- 2026-11-14: wydanie 0.5.1 — język interfejsu (T4.11); testy workspace 146/0 przed wydaniem.
- 2026-11-14: wydanie 0.5.1 opublikowane i sprawdzone: buildy i CI na 3 systemach OK, 6 plików wydania, skróty w
  `.deb` z polskimi wariantami `Comment[pl]`.
- 2026-11-14: README — zrzut okna (`doc/images/bosman8-full.png`): tryb full, SOKOBAN z dyskietki GRY na ekranie
  terminala, panel po angielsku. Do jego zrobienia dodana opcja `--type TEXT` w trybie full (jak w trybie terminal:
  wpisuje tekst na klawiaturze terminala po starcie; `computer --no-gui` ją przyjmuje i ignoruje). `.deb` instaluje
  też `doc/images/`.
- 2026-11-14: wydanie 0.5.2 — zrzut okna w README, `--type` w trybie full, obrazki w `.deb`; testy workspace 146/0.
