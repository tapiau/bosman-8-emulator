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

Specyfikacje łącza: `none`, `loopback`, `stdio`, `tcp-listen:ADDR:PORT`,
`tcp:HOST:PORT`, `serial:DEV[@BAUD]`. Pełna lista opcji: `--help`.

Klawisze terminala: F12 (trzymany) = BREAK, F11 = reset, F10 = CAPS, F9 = turbo,
F8 / Scroll Lock = STOP (stronicowanie), F7 = LOCAL wł./wył. Te same funkcje mają
przyciski pod ekranem (RESET, BREAK, LOCAL, STOP, CAPS, TURBO — klikane myszą,
przełączniki podświetlone, gdy aktywne) i kontrolki LINK / BEEP / WAIT.

## Dokumentacja

- [doc/ARCHITECTURE.md](doc/ARCHITECTURE.md) — architektura, model łącza
- [doc/TASKS.md](doc/TASKS.md) — plan zadań i dziennik postępu
- [doc/bosman8-hardware.md](doc/bosman8-hardware.md) — sprzęt Bosmana ustalony z ROM-u
- [doc/ang3001-hardware.md](doc/ang3001-hardware.md) — sprzęt terminala

ROM-y (`roms/`) pochodzą z repozytoriów analizy `bosman-8-rom` i `bosman-8-ang-3000-rom`.

## Wydania (Linux / Windows / macOS)

Paczki budują GitHub Actions (`.github/workflows/release.yml`) na maszynach
z Linuksem, Windowsem i macOS (arm64 i x86_64). Każda paczka zawiera
`bosman8`, `ang3001`, `b8disk`, katalogi `roms/`, `disks/`, `doc/` i README —
ROM-y są szukane obok programu, więc wystarczy rozpakować i uruchomić.

Publikacja wydania:

```sh
git tag v0.1.0
git push origin v0.1.0     # → workflow buduje paczki i tworzy Release na GitHubie
```

Bez taga: zakładka Actions → Release → „Run workflow” buduje paczki jako
artefakty (do pobrania z przebiegu), bez publikowania. Każdy push uruchamia też
`ci.yml` (testy na trzech systemach).

macOS: programy nie są podpisane — przy pierwszym uruchomieniu
`xattr -d com.apple.quarantine bosman8 ang3001 b8disk` albo „Otwórz” z menu
kontekstowego w Finderze.
