# Bosman-8 + ANG-3001 — emulator

Emulatory polskiego mikrokomputera **Bosman-8** (DZM-180, Z80A, CPM-R) i jego
terminala graficznego **ANG-3001** (Z80B, 512×256). Dwa osobne programy
połączone wirtualnym łączem szeregowym (domyślnie TCP); każdy kanał szeregowy
można też przepiąć na port szeregowy hosta, stdio albo zostawić pusty.

## Szybki start

```sh
cargo build --release
./target/release/bosman8          # okno Bosmana; terminal nasłuchiwany na tcp 127.0.0.1:3001
./target/release/ang3001          # okno terminala, łączy się z 127.0.0.1:3001
```

Na ekranie terminala pojawi się baner i znak zachęty `A>`.

Przykłady:

```sh
bosman8 --no-gui --term stdio                 # bez okien: konsola zamiast terminala (surowe ESC)
bosman8 --disk-b b.img --printer lpt.txt      # dyskietka B: (tworzona, jeśli nie istnieje) i drukarka do pliku
bosman8 --aux serial:/dev/ttyUSB0@9600        # SIO-B (V.24) na prawdziwy port szeregowy
ang3001 --link serial:COM3@115200             # terminal na port szeregowy hosta
ang3001 --load-ter ../bosman-8-ang-3000-rom/demo/cube/CUBE.TER   # demo wgrane wprost do terminala
```

### Okno Bosmana (GUI)

`bosman8` domyślnie otwiera okno (eframe/egui) — panel przedni komputera:

- **Start/Stop**, **RESET** (jak przycisk na obudowie: zerowany tylko CPU, RAM i zatrzaski
  zostają — zob. „Reset” w [doc/bosman8-hardware.md](doc/bosman8-hardware.md)),
  **Wyłącz/włącz** (zimny start z banerem, stosuje zworki), NMI, **Turbo**; status: MHz, czas
  pracy, PC, zatrzaski/bank;
- **stacje B: i C:** — Włóż… (wybór obrazu), Wysuń, Nowa… (pusta dyskietka 200/400/800 KB),
  zabezpieczenie przed zapisem, dioda aktywności, kontrola rozmiaru obrazu wobec formatu ze
  zworek. Dyskietki można zmieniać w czasie pracy (po zmianie `^C` w CP/M); zmiany są
  zapisywane do pliku co sekundę, przy wysunięciu i przy wyjściu. Obraz o złym rozmiarze
  wkładany jest jako zabezpieczony przed zapisem;
- **zworki portu 0x98** (bity 3/5 = format dyskietek, bit 6 = brak C:) — działają po „Wyłącz/włącz”;
- **SIO-A (terminal) i SIO-B (V.24)**: brak / TCP nasłuch / TCP połączenie / port szeregowy
  hosta (lista portów, prędkość) / dowolna specyfikacja; „Zastosuj” przełącza łącze bez
  zatrzymywania maszyny;
- **port równoległy (Centronics)**: plik (dopisywanie/nadpisywanie), łącze `b8-link`
  (np. `tcp:drukarka:9100`, `serial:/dev/ttyUSB0@9600`) albo urządzenie hosta (`/dev/usb/lp0`);
  licznik wydrukowanych bajtów;
- podgląd surowego wyjścia SIO-A (ESC jako `^[`).

Ustawienia zapisują się w `~/.config/bosman8/gui.json` (Windows: `%APPDATA%\bosman8`,
macOS: `~/Library/Application Support/bosman8`; inny plik: `--config PATH`). Opcje
`--rom --term --aux --dip --disk-b --disk-c --printer --turbo` nadpisują zapisane ustawienia.
`--no-gui` uruchamia dotychczasowy tryb konsolowy (wszystkie opcje jak dawniej). Build bez okna:
`cargo build -p bosman8 --no-default-features` (wtedy zawsze tryb konsolowy).

Dyskietka z narzędziami CP/M (PIP, STAT, ED, ZSID, …): `disks/narzedzia.img`, zob.
[disks/README.md](disks/README.md). Obrazy edytuje `b8disk` (`b8disk put b.img PLIK.COM`, `b8disk ls b.img`).

Obrazy z Gotka (`.hfe`, HxC v1) działają bezpośrednio — w `--disk-b/--disk-c`, w „Włóż…”
i w `b8disk` — bez konwersji. Zapis zmienia w pliku tylko pole danych zapisanych sektorów
(z nowym CRC), reszta ścieżek zostaje bit w bit, więc plik można od razu wgrać z powrotem na
pendrive Gotka. „Nowa…” / `b8disk new` z nazwą `*.hfe` tworzy pustą dyskietkę HFE. Jeśli w
pliku brakuje sektorów formatu wybranego zworkami (np. dyskietka 200 KB przy zworkach 400 KB),
obraz jest wkładany jako zabezpieczony przed zapisem, a komunikat podpowiada właściwy format.

Specyfikacje łącza: `none`, `loopback`, `stdio`, `tcp-listen:ADDR:PORT`,
`tcp:HOST:PORT[,raw]`, `serial:DEV[@BAUD][,FORMAT]`. Pełna lista opcji: `--help`.

Port szeregowy otwiera się jako 8N1 (albo z formatem podanym po przecinku, np.
`serial:/dev/ttyUSB0@100000,8N2`); bez `--fixed-baud` emulowany układ przestawia prędkość i format,
gdy ROM go zaprogramuje (łącze Bosman ↔ ANG: 100000 8N2). Z `--fixed-baud` port zostaje przy
tym, co podano w specyfikacji — dla ANG-3001 trzeba więc dopisać `,8N2`.

Między emulatorami po TCP działa sprzętowe sterowanie przepływem (RTS/CTS, podzbiór RFC 2217):
terminal, który nie nadąża rysować, wstrzymuje Bosmana — jak na prawdziwym sprzęcie (SIO-A
pracuje z Auto Enables). Bez tego gry rysujące dużo grafiki (np. SOKOBAN) reagowały na klawisze
z opóźnieniem dziesiątek sekund. Zwykły klient TCP (`nc`, program terminalowy) dalej dostaje
surowy strumień bajtów; `tcp:HOST:PORT,raw` łączy bez protokołu (np. z `ser2net` w trybie raw).
Na porcie szeregowym RTS/CTS włącza `,rtscts` (np. `serial:/dev/ttyUSB0@100000,8N2,rtscts`, w GUI
Bosmana checkbox „RTS/CTS”): emulowany terminal steruje linią RTS portu, a emulowany Bosman czyta
CTS i wstrzymuje nadawanie. Tylko z kablem, który ma linię CTS — na kablu 3-żyłowym niepodłączone
wejście CTS czyta się jako „stop” i emulator czekałby w nieskończoność, dlatego domyślnie wyłączone.

Klawisze terminala: F12 (trzymany) = BREAK, F11 = reset, F10 = CAPS, F9 = turbo,
F8 / Scroll Lock = STOP (stronicowanie), F7 = LOCAL wł./wył. Strzałki domyślnie wysyłają
cyfry 8 / 2 / 4 / 6 (góra / dół / lewo / prawo — jak klawiatura numeryczna, tego używają gry, np.
SOKOBAN); tryb zmienia lista ARROWS na pasku albo F6 (2468, VT52 = `ESC A–D`, WordStar = ^E ^X ^S ^D,
VIM = k j h l, NONE), a przy starcie `--arrows numpad|vt52|wordstar|vim|none` (`off` = `none`). Te same funkcje mają
przyciski pod ekranem (RESET, BREAK, LOCAL, STOP, CAPS, TURBO — klikane myszą,
przełączniki podświetlone, gdy aktywne) i kontrolki LINK / BEEP / WAIT.

ROM terminala: domyślnie wbudowany ROM 90.09.11 (`roms/ang3001/ANG-3001_90.09.11.BIN`) — zna
sekwencje sterujące, których używają gry z dyskietki GRY (np. SOKOBAN rysuje planszę). BREAK od
razu wysyła BREAK na łącze, a LOCAL włącza się przyciskiem / F7 (bez zachęty „BREAK - powtórz”).
Starszy ROM: `ang3001 --rom roms/ang3001/ANG3001_ZJ_wersjaPG.BIN` (na nim te gry wypisują kody).
Emulator rozpoznaje wersję ROM-u sam. ROM Bosmana: domyślnie `roms/bosman8/bosman8.bin` (CPM-R,
32 KB), a gdy pliku nie ma obok — ta sama wersja wbudowana w program; obrazy UNIMOR ZJ (16 KB) tylko
przez `--rom`.

## Dokumentacja

- [doc/ARCHITECTURE.md](doc/ARCHITECTURE.md) — architektura, model łącza
- [doc/TASKS.md](doc/TASKS.md) — plan zadań i dziennik postępu
- [doc/bosman8-hardware.md](doc/bosman8-hardware.md) — sprzęt Bosmana ustalony z ROM-u
- [doc/ang3001-hardware.md](doc/ang3001-hardware.md) — sprzęt terminala

ROM-y pochodzą z repozytoriów analizy `bosman-8-rom` i `bosman-8-ang-3000-rom`, po katalogu na maszynę:
`roms/bosman8/` (CPM-R `bosman8.bin` 32 KB — domyślny, i obrazy UNIMOR ZJ 16 KB), `roms/ang3001/`
(terminal: `ANG-3001_90.09.11.BIN` — domyślny, `ANG3001_ZJ_wersjaPG.BIN`); miejsce na kolejne
terminale (np. `roms/an2001/`).

## Wydania (Linux / Windows / macOS)

Paczki budują GitHub Actions (`.github/workflows/release.yml`) na maszynach
z Linuksem, Windowsem i macOS (arm64 i x86_64). Każda paczka zawiera
`bosman8`, `ang3001`, `b8disk`, katalogi `roms/`, `disks/`, `doc/`, `tools/` i README —
ROM-y są szukane obok programu, więc wystarczy rozpakować i uruchomić.
`BSRV.COM` jest też osobnym plikiem wydania: serwer D/E/F dla prawdziwych
Bosmanów z ROM-ami UNIMOR ZJ, z [instrukcją instalacji](tools/bsrv/README.md).

Publikacja wydania:

```sh
git tag v0.4.3
git push origin v0.4.3     # → workflow buduje paczki i tworzy Release na GitHubie
```

Bez taga: zakładka Actions → Release → „Run workflow” buduje paczki jako
artefakty (do pobrania z przebiegu), bez publikowania. Każdy push uruchamia też
`ci.yml` (testy na trzech systemach).

Paczka Debiana/Ubuntu (`bosman8-emulator_X.Y.Z_amd64.deb`) jest dołączana do
każdego wydania; lokalnie: `cargo build --release && packaging/debian/build-deb.sh`.
Instaluje programy do `/usr/bin`, ROM-y i dyskietkę do `/usr/share/bosman8-emulator`,
skróty w menu, a ścieżki ROM-ów w `/etc/bosman-8-emulator.conf` (klucze
`bosman8_rom`, `ang3001_rom`; plik konfiguracyjny — aktualizacja go nie nadpisze).
Dyskietkę z `/usr/share` skopiuj do katalogu domowego przed użyciem (emulator
zapisuje zmiany do obrazu).

macOS: programy nie są podpisane — przy pierwszym uruchomieniu
`xattr -d com.apple.quarantine bosman8 ang3001 b8disk` albo „Otwórz” z menu
kontekstowego w Finderze.
