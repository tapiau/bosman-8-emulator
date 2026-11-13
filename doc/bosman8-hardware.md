# Bosman-8 — sprzęt z punktu widzenia emulatora

Źródło: `../bosman-8-rom` (ROM `Bosman8-kompatybilny_z_ANG-3001_SAJ.BIN`, 32 KB)
+ własna analiza kodu. Tam gdzie dokumentacja w `bosman-8-rom/doc` jest sprzeczna
z kodem, obowiązuje to, co poniżej (z uzasadnieniem).

## CPU
Z80A @ 4 MHz. Reset → 0x0000 (`25 00` = `DEC H; NOP`, potem `JP 0255h`).
NMI → 0x0066 → `JP F341` (wektor w RAM). Przerwania IM1 → 0x0038 (`JP F272`).

## Zatrzask 74LS259 — porty 0x00–0x0F  🟢
Wzorzec dostępu jednoznacznie wskazuje adresowalny zatrzask: **numer bitu =
port >> 1, wartość = port & 1**, dana na szynie jest ignorowana. Po resecie
wszystkie bity = 0.

| Bit | Porty (0/1) | Znaczenie | Dowód |
|---|---|---|---|
| Q0 | 00/01 | ❓ nieznane (zob. niżej) | `OUT(01);LDIR;OUT(00)` w 0x005E |
| Q1 | 02/03 | nieużywany | |
| Q2 | 04/05 | nieużywany | |
| Q3 | 06/07 | ❓ wskaźnik/LED testu ROM (1 podczas liczenia sumy) | boot 0x0255 / 0x0267 |
| Q4 | 08/09 | bit 1 numeru banku RAM | `F30F` |
| Q5 | 0A/0B | bit 2 numeru banku RAM | `F30F` |
| Q6 | 0C/0D | bit 0 numeru banku RAM | `F30F` |
| Q7 | 0E/0F | 0 = ROM widoczny w 0000–7FFF, 1 = RAM | trampoliny `F006`/`F00B`/`F060` |

Procedura `F30F` (kopia z ROM 0x2E0F): `C=6: RRCA, RL C, OUT (C),A` itd. —
rozkłada 3 młodsze bity A na Q6, Q4, Q5. A=FF → bank domyślny z `F26B`.

## Pamięć  🟡
- **0x0000–0x7FFF**: odczyt — ROM (gdy Q7=0) albo RAM wybranego banku (Q7=1).
  **Zapis zawsze idzie do RAM wybranego banku** (także gdy ROM jest widoczny) —
  kod RAM-dysku (0x085D) wykonywany z ROM robi `LDIR` do adresów < 0x8000 po
  wybraniu banku, a `READ_CONFIG` (0x03FF) zapisuje markery pod adres 0 przy
  widocznym ROM i odczytuje je przez `F458` (`OUT(0F); LD A,(HL)`).
- **8 banków × 32 KB** (3 bity). Bank 0 = TPA.
- **0x8000–0xFFFF**: stałe 32 KB RAM (BIOS w F200, zmienne, bufory).
- `READ_CONFIG` testuje banki 0/1 i 4/5; `F437 = 2*R(4)+R(0)-1`. Przy pełnej
  pamięci R=2 → F437=5 → ścieżka z banerem. Emulator domyślnie: 8 banków.
- Odczyt adresu 0 jako detekcja stanu: `25h` = ROM (Q0=0), `A5h` = ROM przy
  Q0=1 (procedura `F495`). Hipoteza: Q0 przełącza drugą połowę EPROM-u / drugi
  ROM, którego nie mamy. Emulator: Q0 ignorowany, logowany w `--trace-io`.

## Z80-SIO — porty 0x80–0x83  🟢
80 = dane A, 81 = dane B, 82 = sterowanie A, 83 = sterowanie B
(układ „C/D na A0, A/B na A1”).
- Kanał A = **terminal** (ANG-3001), tryb synchroniczny ~100 kbit/s
  (8253 licznik 1 = 20 → 100 kHz).
- Kanał B = V.24 (drugi komputer, PUNCH, napędy D/E/F), async ×16, domyślnie
  ~9600 (8253 licznik 2 = 13).
- Używane rejestry: RR0 bit0 (Rx avail), bit2 (Tx empty), bit7 (break/abort,
  po `OUT(82),10h`); RR1 bit0 (all sent) — BIOS CONOUT `F3CE` czeka na RR1.0
  zanim wyśle znak. WR0 bity 0–2 wybierają rejestr, komendy w bitach 3–5.

## 8253 — porty 0x84–0x87  🟢
Zegar 2 MHz. Licznik 0 (tryb 2, BCD, 0x9999) — timing FDC/opóźnienia,
licznik 1 — zegar SIO-A, licznik 2 — zegar SIO-B. Emulator: pełne liczniki
z odczytem (latch), bez wyjść poza informacją o częstotliwości dla SIO.

## WD1770 — porty 0x88–0x8B  🟡
Status/komenda, ścieżka, sektor, dane. Porty 0xF0–0xFF wyglądają na drugi
zatrzask 74LS259 (wybór napędu, strona, silnik — F2/F3, F4/F5, F8, FE/FF).
Format dyskietki: DPB 40 sektorów logicznych/ścieżkę × 128 B, 200 KB.
❓ fizyczny układ sektorów do ustalenia z kodu 0x0670–0x0E80.

## Port 0x98  🟡
IN: zworki — bity 3/5 wybierają tablicę DPB dyskietki (0x79/0xA1/0xC9),
bit 6 = 0 → dodatkowa inicjalizacja FDC (`HW_INIT_1`). Pozostałe bity
prawdopodobnie status drukarki. Emulator: konfigurowalna wartość, domyślnie
`0x40`. OUT: dane Centronics.

## Inne porty widziane w kodzie
0xF5, 0xF8, 0xFF, 0xFE, 0xF2–0xF7 (FDC/napędy), 0x9C (IN, raz). Na razie
ignorowane i logowane.
