# Bosman-8 + ANG-3001 — emulator

Emulator polskiego mikrokomputera **Bosman-8** (DZM-180, Z80A, CPM-R) i jego
terminala graficznego **ANG-3001** (Z80B, 512×256) — jeden program, `bosman8`,
w trzech trybach:

| tryb | co działa | do czego |
|---|---|---|
| `bosman8` (= `bosman8 full`) | komputer i terminal w jednym oknie, połączone w pamięci | zwykłe używanie |
| `bosman8 computer` | sam komputer; łącze terminala (SIO-A) na TCP / porcie szeregowym / stdio | prawdziwy terminal ANG-3001, testy |
| `bosman8 terminal` (= `ang3001`) | sam terminal; łącze na TCP / porcie szeregowym | prawdziwy Bosman |

Uruchomione osobno `bosman8 computer` i `bosman8 terminal` łączą się same po TCP
127.0.0.1:3001 — dwa okna, jak w wersjach sprzed 0.5.

## Szybki start

```sh
cargo build --release
./target/release/bosman8          # jedno okno: ekran terminala + panel ustawień komputera
```

Na ekranie terminala pojawi się baner i znak zachęty `A>`; klawiatura pisze do terminala.

Przykłady:

```sh
bosman8 --disk-b b.img --printer lpt.txt      # dyskietka B: (tworzona, jeśli nie istnieje) i drukarka do pliku
bosman8 computer --term serial:/dev/ttyUSB0@100000,8N2,rtscts   # prawdziwy ANG-3001 na porcie szeregowym
bosman8 computer --no-gui --term stdio        # bez okna: konsola zamiast terminala (surowe ESC)
bosman8 --aux serial:/dev/ttyUSB0@9600        # SIO-B (V.24) na prawdziwy port szeregowy
bosman8 terminal --link serial:COM3@100000,8N2   # sam terminal do prawdziwego Bosmana
bosman8 terminal --load-ter ../bosman-8-ang-3000-rom/demo/cube/CUBE.TER   # demo wgrane wprost do terminala
ang3001 --headless --after 3 --dump-text      # dawne wywołanie terminala działa (dowiązanie)
```

### Okno

Wspólne dla trybów (eframe/egui): w centrum ekran terminala (proporcja 4:3), po prawej
panel ustawień chowany przyciskiem ⚙ (stan zapamiętany), na dole pasek z przyciskami
terminala (RESET, BREAK, LOCAL, STOP, CAPS, lista ARROWS — jak dawniej pod ekranem
`ang3001`), lampkami LINK / BEEP / WAIT, lampkami stacji B: / C:, przyciskami RESET i
Wyłącz/włącz komputera oraz TURBO (wspólne dla komputera i terminala, F9). Klawiatura idzie
do terminala, chyba że edytujesz pole tekstowe w panelu. W trybie `full` SIO-A ustawione na
„wbudowany terminal ANG-3001”; po przełączeniu SIO-A na inne łącze w centrum widać stan łącza
i podgląd wyjścia SIO-A. W trybie terminal panel zawiera „Łącze terminala” (TCP / port
szeregowy, RTS/CTS) i sekcję „Terminal ANG-3001” (ROM, kolor, pierwsza linia obrazu).

Ikona programu (koło sterowe z „8”) jest w `packaging/icons/` — źródła SVG i `render.sh`, który
robi z nich PNG, `.ico` (wbudowane w `bosman8.exe`) i ikonę okna.

Panel komputera (tryby `full` i `computer`):

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

Ustawienia zapisują się osobno dla każdego trybu — `full.json`, `computer.json`,
`terminal.json` w `~/.config/bosman8/` (Windows: `%APPDATA%\bosman8`, macOS:
`~/Library/Application Support/bosman8`; inny plik: `--config PATH`), więc dwa okna naraz
nie nadpisują sobie ustawień; dawny `gui.json` jest przy pierwszym uruchomieniu kopiowany do
nowych plików. Opcje `--rom --term --aux --dip --disk-b --disk-c --printer --turbo`
(oraz `--terminal-rom --arrows` w trybie `full`) nadpisują zapisane ustawienia; opcje trybu
terminal — `bosman8 terminal --help` (te same co dawnego `ang3001`). `bosman8 computer --no-gui`
(także samo `bosman8 --no-gui`) uruchamia komputer w wierszu poleceń, `bosman8 terminal
--headless` — terminal bez okna (zrzuty ekranu, `--dump-text`). Build bez okna:
`cargo build -p bosman8 --no-default-features`.

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
przyciski na pasku pod ekranem (RESET, BREAK, LOCAL, STOP, CAPS, TURBO — przełączniki
podświetlone, gdy aktywne) i lampki LINK / BEEP / WAIT.

ROM terminala: domyślnie wbudowany ROM 90.09.11 (`roms/ang3001/ANG-3001_90.09.11.BIN`) — zna
sekwencje sterujące, których używają gry z dyskietki GRY (np. SOKOBAN rysuje planszę). BREAK od
razu wysyła BREAK na łącze, a LOCAL włącza się przyciskiem / F7 (bez zachęty „BREAK - powtórz”).
Starszy ROM: `bosman8 terminal --rom roms/ang3001/ANG3001_ZJ_wersjaPG.BIN` albo w panelu „Terminal
ANG-3001” → Wybierz… (na nim te gry wypisują kody).
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
`bosman8`, `b8disk`, `ang3001` (dowiązanie do `bosman8`; w Windows `ang3001.cmd` =
`bosman8 terminal`), katalogi `roms/`, `disks/`, `doc/`, `tools/` i README —
ROM-y są szukane obok programu, więc wystarczy rozpakować i uruchomić.
`BSRV.COM` jest też osobnym plikiem wydania: serwer D/E/F dla prawdziwych
Bosmanów z ROM-ami UNIMOR ZJ, z [instrukcją instalacji](tools/bsrv/README.md).

Publikacja wydania:

```sh
git tag v0.5.0
git push origin v0.5.0     # → workflow buduje paczki i tworzy Release na GitHubie
```

Bez taga: zakładka Actions → Release → „Run workflow” buduje paczki jako
artefakty (do pobrania z przebiegu), bez publikowania. Każdy push uruchamia też
`ci.yml` (testy na trzech systemach).

Paczka Debiana/Ubuntu (`bosman8-emulator_X.Y.Z_amd64.deb`) jest dołączana do
każdego wydania; lokalnie: `cargo build --release && packaging/debian/build-deb.sh`.
Instaluje programy do `/usr/bin` (`ang3001` jako dowiązanie do `bosman8`), ROM-y i dyskietkę do
`/usr/share/bosman8-emulator`, skróty w menu („Bosman-8”, „Bosman-8 (computer)”, „ANG-3001
(terminal)”), a ścieżki ROM-ów w `/etc/bosman-8-emulator.conf` (klucze
`bosman8_rom`, `ang3001_rom`; plik konfiguracyjny — aktualizacja go nie nadpisze).
Dyskietkę z `/usr/share` skopiuj do katalogu domowego przed użyciem (emulator
zapisuje zmiany do obrazu).

macOS: programy nie są podpisane — przy pierwszym uruchomieniu
`xattr -d com.apple.quarantine bosman8 b8disk` albo „Otwórz” z menu
kontekstowego w Finderze.
