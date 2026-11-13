# Bosman-8 — sprzęt z punktu widzenia emulatora

Źródło: `../bosman-8-rom` (ROM `Bosman8-kompatybilny_z_ANG-3001_SAJ.BIN`, 32 KB)
+ własna analiza kodu. Tam gdzie dokumentacja w `bosman-8-rom/doc` jest sprzeczna
z kodem, obowiązuje to, co poniżej (z uzasadnieniem).

Pewność: 🟢 potwierdzone (kod jednoznaczny i emulator z tym modelem bootuje /
działa), 🟡 prawdopodobne, ❓ hipoteza / nieznane.

**Stan emulacji (T2.1–T2.6):** ROM bootuje do `A>` z banerem „Mikrokomputer …
RAMDYSK 408 KB”, `DIR` na RAM-dysku odpowiada „Brak pliku”, `SAVE`/`DIR` na
dyskietkach B: i C: działają (obraz zapisuje się poprawnie), `^P` drukuje na
Centronics. Wszystko to bez żadnych łatek ROM-u — model poniżej wystarcza.

## CPU
Z80A @ 4 MHz. Reset → 0x0000 (`25 00` = `DEC H; NOP`, potem `JP 0255h`).
NMI → 0x0066 → `JP F341` (wektor w RAM). Przerwania IM1 → 0x0038 (`JP F272`).

## Zatrzask 74LS259 — porty 0x00–0x0F  🟢
Wzorzec dostępu jednoznacznie wskazuje adresowalny zatrzask: **numer bitu =
port >> 1, wartość = port & 1**, dana na szynie jest ignorowana. Po resecie
wszystkie bity = 0.

| Bit | Porty (0/1) | Znaczenie | Dowód |
|---|---|---|---|
| Q0 | 00/01 | 🟡 A15 EPROM-u (górna połowa 64 KB), zob. niżej | `OUT(01);LDIR;OUT(00)` w 0x005E |
| Q1 | 02/03 | nieużywany | |
| Q2 | 04/05 | nieużywany | |
| Q3 | 06/07 | 🟡 LED błędu: `OUT(07)` na starcie, `OUT(06)` gdy suma ROM = 0; przy złej sumie zostaje 1 | boot 0x0255 / 0x0267 |
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
  Q0=1 (procedura `F495` = ROM 0x2F95). 🟡 **Q0 = linia A15 EPROM-u 27512**:
  - `0x00F1` (wektor 0x001A): kopiowanie `LDIR` z ROM; gdy HL ma bit 7 = 1,
    kod robi `RES 7,H; OUT(01); LDIR; OUT(00)` — czyli „adres ROM 8000–FFFF” to
    druga połowa EPROM-u widziana w 0000–7FFF przy Q0=1;
  - `F481` (ROM 0x2F81, wołane z wektora BIOS `F251`): `OUT(01)`, ROM widoczny,
    skok do procedury w drugiej połowie; powrót przez `F485`/`F495`, które
    po bajcie spod adresu 0 (25/A5/inne) odtwarzają stan Q0/Q7.
  Nasz zrzut ma 32 KB (27256 albo tylko dolna połowa). Przy 27256 A15 nie jest
  podłączone i Q0 nic nie zmienia — tak też działa emulator z obrazem 32 KB.
  Emulator przyjmuje też obraz 64 KB: wtedy Q0=1 wybiera jego górną połowę.
  Zmiany Q0 są logowane w `--trace-io`. Podczas bootu i pracy CCP Q0 nie jest
  używany.

## Z80-SIO — porty 0x80–0x83  🟢
80 = dane A, 81 = dane B, 82 = sterowanie A, 83 = sterowanie B
(układ „C/D na A0, A/B na A1”).
- Kanał A = **terminal** (ANG-3001), tryb synchroniczny ~100 kbit/s
  (8253 licznik 1 = 20 → 100 kHz). WR3=E1, WR4=0C (sync, ×1), WR5=E8.
- Kanał B = V.24 (drugi komputer, PUNCH, napędy D/E/F), async ×16, domyślnie
  ~9600 (8253 licznik 2 = 13).
- Używane rejestry: RR0 bit0 (Rx avail), bit2 (Tx empty), bit7 (break/abort,
  po `OUT(82),10h`); RR1 bit0 (all sent) — BIOS CONOUT `F3CE` czeka na RR1.0
  zanim wyśle znak. WR0 bity 0–2 wybierają rejestr, komendy w bitach 3–5.
- 🟢 Emulacja: model bajtowy z `b8-chips` wystarcza, przerwania SIO nie są
  używane (CONIN/CONOUT odpytują RR0/RR1). Czas znaku liczony z częstotliwości
  wyjścia 8253 (licznik 1 → A, 2 → B), dzielnika z WR4 i długości ramki:
  kanał A sync 8 bitów → 100 kHz/8 = 12,5 kB/s = **320 cykli/bajt**; kanał B
  async ×16, 8N1 (WR4=4C → 1 bit stopu) → 153,8 kHz/16/10 ≈ 961 B/s
  ≈ 4160 cykli/bajt. Wysyłany BREAK (WR5 bit 4) jest przekazywany do endpointu;
  odbieranego BREAK endpointy na razie nie zgłaszają.
- 🟢 Odbiór (T4.4): 3-bajtowe FIFO jak w Z80-SIO, nadmiar czeka w kolejce
  hosta (bez przepełnień); po zapisie licznika 8253 danego kanału odbiornik
  przez 3 ms nie pobiera bajtów z kolejki (`SIOB_INIT` 0x1487 wyrzuca to, co
  przyjdzie w ciągu 2 ms po zmianie prędkości, a bajty z TCP przychodzą
  paczkami); SIO-B gubi bajty, gdy odbiornik jest wyłączony (WR3 bit 0).
  Szczegóły i uzasadnienie: [bosman8-interlink.md](bosman8-interlink.md).

## 8253 — porty 0x84–0x87  🟢
Zegar 2 MHz. Licznik 0 (tryb 2, BCD, 0x9999) — timing FDC/opóźnienia,
licznik 1 — zegar SIO-A, licznik 2 — zegar SIO-B. Emulator: pełne liczniki
z odczytem (latch), bez wyjść poza informacją o częstotliwości dla SIO.

## WD1770 — porty 0x88–0x8B  🟢
Status/komenda, ścieżka, sektor, dane. **Przesył danych jest odpytywany przez
port 0x98 bit 0 (= /DRQ), a koniec komendy sygnalizuje NMI** (INTRQ → NMI):

```
0B17  OUT (F3)          ; Q1 drugiego zatrzasku := 1 — odblokuj NMI
0B27  LD A,80h / OUT (88h),A   ; READ SECTOR
0B2B  IN A,(98h) / RRA / JP C,0B2B   ; czekaj na DRQ (bit0 = 0)
0B31  INI / JP 0B2B     ; pętla bez wyjścia — kończy ją dopiero NMI
NMI → 0066 JP F341 → JP (F342) = 0B45: INC SP;INC SP (zrzuca adres powrotu NMI),
      IN A,(88h) (status, kasuje INTRQ), OUT (F2) (Q1 := 0), ...
```

Tak samo RESTORE (0x0E07, handler 0x0E5D) i SEEK (0x0D15, handler 0x0D6F):
`F342` = adres handlera, potem pętla `OUT(F2); opóźnienie; OUT(F3)` z licznikiem
— jeśli NMI nie nadejdzie, komunikat „H-ERROR - mikrosystem (M) ignoruj (I)”
(RESTORE, ~10 s) albo „Sprawdź czy jest dyskietka” (SEEK).

🟢 **NMI = INTRQ AND Q1(zatrzask 2)**, wyzwalane zboczem (Z80 NMI jest zboczowe;
`OUT(F3)` przy już aktywnym INTRQ też daje NMI — na tym opiera się pętla
`F2/opóźnienie/F3`). Emulator: NMI przy narastającym zboczu tego iloczynu,
sprawdzane po każdej instrukcji.

### Drugi zatrzask 74LS259 — porty 0xF0–0xFF  🟡
Ten sam układ adresowania co 0x00–0x0F (bit = (port>>1)&7, wartość = port&1).

| Bit | Porty | Znaczenie | Dowód | Pewność |
|---|---|---|---|---|
| Q1 | F2/F3 | blokada/odblokowanie NMI od INTRQ | handlery NMI zaczynają od `OUT(F2)`; `OUT(F3)` przed komendą | 🟢 |
| Q2 | F4/F5 | `OUT(F5)` przy starcie, impuls `F4`–5 ms–`F5` gdy napęd nie gotowy / rezygnacja (0x0DF7) — może /MR WD1770 albo reset napędu | 0x02FE, 0x0DF7 | ❓ (emulator: tylko log) |
| Q3 | F6/F7 | strona dyskietki (1 = strona 1) | 0x0D0A | 🟡 |
| Q4 | F8/F9 | wybór napędu 1 = **C:** | tablica portów `F339/F33A` = `FF`,`F9` | 🟢 |
| Q7 | FE/FF | wybór napędu 0 = **B:** | j.w.; 0x0CBD odznacza oba (`F8`,`FE`) | 🟢 |
| Q0, Q5, Q6 | | nieużywane przez ROM | | |

Napęd: przed operacją `OUT(F8); OUT(FE)` (odznacz oba), potem `OUT (C)` z
portem z `F339+napęd-1`. Silnik — WD1770 steruje nim sam (MO).

### Format dyskietek  🟢 (z kodu, zweryfikowany zapisem `SAVE` na B: i C:)
Parametry dysków to 20-bajtowe wpisy (napęd B:, napęd C:) kopiowane do
`F2BF`/`F2D3`: 15 bajtów DPB CP/M + `PSH` + `PHM` + numer pierwszego sektora
strony 0 + strony 1 + bajt flag (kopiowane przez 0x0A62 do `F33B..F33D`).

- `PSH = 3` → **sektory fizyczne 1024 B**, `SPT>>PSH = 40>>3 = 5` sektorów na
  ścieżkę (= 40 sektorów logicznych po 128 B). Sektory numerowane od
  `first_sector` (ten sam na obu stronach), indeks 0..4 (0x0A90, 0x0D04).
- Flagi: bity 0–1 = szybkość kroku WD1770 (RESTORE/SEEK); bit 7 = podwójny krok
  (rejestr ścieżki ×2 przy SEEK; nieużywany w tablicach ROM); bit 6 przy bicie
  4 = 0 → **strony naprzemiennie**: ścieżka logiczna t → cylinder t/2, strona t&1;
  w przeciwnym razie ścieżki ≥ 40 (≥ 80 przy bicie 5) idą na stronę 1, ścieżka
  −40/−80 (0x0CD9–0x0CF6). Po SEEK rejestr ścieżki = cylinder (pole ID ścieżki
  = cylinder fizyczny).

Wybór tablicy zworkami portu 0x98 (`AND 28h`, 0x02DC):

| bity 5,3 | tablica | B: | C: |
|---|---|---|---|
| 00 | wbudowana w obraz RAM (ROM 0x2DBF) | 200 KB: 40 cyl × 1 str., sektory 11h–15h | 200 KB, j.w. |
| 08 | 0x0079 | 400 KB: 40 cyl × 2 str., 21h–25h | 400 KB, j.w. |
| 20 | 0x00A1 | 800 KB: 80 cyl × 2 str., 31h–35h | 800 KB, j.w. |
| 28 | 0x00C9 | 800 KB: 80 × 2, 31h–35h, krok 30 ms | 200 KB: 40 × 1, 11h–15h |

Obraz dyskietki w emulatorze: surowe sektory w kolejności cylinder, strona,
sektor (dla formatów dwustronnych = kolejność ścieżek logicznych CP/M), np.
204 800 B dla 200 KB. Brakujący plik jest tworzony jako pusty (0xE5).
OFF = 0 (brak ścieżek systemowych), katalog 64/192 wpisy.

### Wykrywanie napędów przy starcie (0x0DB7)  🟢
`HW_INIT(A)`: A=0 → B:, A=1 → C: (wołane z A=1 tylko gdy bit 6 portu 0x98 = 0).
Wybiera napęd, RESTORE z NMI; jeśli po RESTORE status ma TR00 → napęd jest
(`F370` bit 1 = B:, bit 2 = C:). Dla B: czeka potem do 2,5 s na bit 2 portu 0x98
(gotowość); gdy brak — impuls Q2 (`F4`/`F5`), gdy jest — `FB7E` bit 1.
Uwaga: domyślnie (DIP 0x40) **C: nie jest inicjalizowany i nie istnieje** —
emulator przy `--disk-c` bez jawnego `--dip` zeruje bit 6.
Emulowany napęd B: istnieje zawsze (TR00 działa bez dyskietki), więc bez
`--disk-b` start trwa ~2,5 s dłużej (oczekiwanie na gotowość) — jak na
prawdziwym sprzęcie z pustą stacją.

## Port 0x98  🟡
IN — mieszanka zworek i sygnałów statusu:

| Bit | Znaczenie | Dowód | Pewność | Emulator |
|---|---|---|---|---|
| 0 | **/DRQ WD1770** (0 = bajt gotowy) | pętle `IN(98);RRA;JP C` + `INI`/`OUTI` 0x0B2B/0x0B3A | 🟢 | z FDC |
| 1 | ? | nieczytany | | z `--dip` |
| 2 | gotowość wybranego napędu (READY ze stacji) | 0x0DEA, 0x0AB2 (czeka, potem „Sprawdź czy jest dyskietka”) | 🟡 | 1 gdy w wybranym napędzie jest obraz |
| 3, 5 | zworki: format dyskietek (tabela wyżej) | 0x02DC | 🟢 | z `--dip` |
| 4 | zajętość urządzenia równoległego PUNCH (czeka na 0) | 0x1254 | 🟡 | 0 |
| 6 | zworka: 0 = jest napęd C: (wołaj `HW_INIT(1)`) | 0x0317 | 🟢 | z `--dip` |
| 7 | drukarka gotowa (1 = może przyjąć znak) / dane czytnika gotowe | 0x105D, 0x11AF, 0x12CE | 🟡 | 1 |

Bit 7 dla innego typu drukarki (konfiguracja IOBYTE/`F26B`, gałąź 0x106C) ma
odwrotną polaryzację i dane idą zanegowane (`CPL`) — emulator obsługuje tylko
domyślny wariant (gotowa = 1, dane proste).

OUT 0x98: dane Centronics (i PUNCH). 🟡 Brak osobnego portu strobu: procedura
czeka na bit 7 = 1, potem tylko `OUT (98h),A` — strob generuje sprzęt przy
zapisie (zapewne przerzutnik kasowany przez ACK/BUSY). Emulator: każdy zapis do
0x98 = jeden bajt do pliku `--printer`.

IN 0x9C: dane czytnika (READER), zanegowane, gdy bit 7 portu 0x98 = 1 (0x12CE).
Nieemulowane (0xFF).

## NMI i przerwania  🟢
NMI → 0x0066 `JP F341` → `F341: JP nnnn` z `F342` ustawianym przed każdą
komendą FDC. Jedynym źródłem NMI jest WD1770 INTRQ (bramkowane Q1 drugiego
zatrzasku). Handlery zdejmują adres powrotu (`INC SP` ×2) i wracają do
procedury, która wydała komendę. IM1 (`0x0038 → F272`) nie jest potrzebne do
bootu ani pracy CCP — emulator nie generuje przerwań maskowalnych.

## Reset  🟡 (model) / ❓ (zgodność z obserwacją)
Obserwacja z prawdziwego sprzętu: RESET przy włączonym terminalu daje tylko
znak zachęty `A>`, bez ramki z banerem. Baner drukuje wyłącznie pełny start ROM
(0x0255 → … → 0x03B1, gdy suma ROM = 0 i `F437` = 5); ciepły start BIOS-u
(`F203` → `F376`: `OUT(00); OUT(0E); JP 03DA`) wchodzi już za kodem banera
(0x03DA → 0x03F0: sekwencja 0x023E bez banera, `JP 3E64` CCP).

Model w emulatorze (`Bosman8::warm_reset`, przycisk RESET w GUI): **reset
zeruje tylko CPU** (i WD1770, MR), RAM i oba zatrzaski 74LS259 zostają. Jeśli
w chwili resetu w 0000 widoczny jest RAM (Q7 = 1, bank 0 — działa program z
TPA), CPU wykonuje `JP F203` → ciepły start → sam znak zachęty (test
`control::tests::warm_reset_in_program_gives_prompt_without_banner`).

❓ Uwaga: przy samym znaku zachęty CCP CPM-R działa **z ROM-u** (zatrzask = 0x70:
Q7 = 0, bank 7; pętla CONIN w `F3A0–F41F`), więc w tym modelu reset przy `A>`
uruchamia pełny start z ROM-u — z banerem (sygnatura 0x55AA w 0x8800 pomija
tylko czyszczenie bufora). Hipoteza „RESET → NMI” też nie pasuje: NMI → 0066 →
`JP F341` → `JP 0E5D` (ostatni handler FDC) wraca do przerwanej pętli i nic nie
drukuje (sprawdzone w emulatorze). Do wyjaśnienia na sprzęcie (w jakim stanie
był komputer przy resecie, schemat linii RESET). Zimny start (`cold_reset`,
„Wyłącz/włącz” w GUI) zeruje CPU, zatrzaski, SIO, 8253, WD1770 i RAM.

## Opóźnienia
`0x2C67 DELAY`: A × (pętla `DJNZ` 244 razy) ≈ A × 1 ms przy 4 MHz — czysto
programowe, więc w trybie `--turbo` też trwa tyle samo czasu emulowanego.
Boot do `A>` w czasie rzeczywistym: ~2,5 s z dyskietką w B:, ~4,8 s bez.

## Inne porty widziane w kodzie
Nieznane porty: odczyt zwraca 0xFF, zapis ignorowany, wszystko logowane w
`--trace-io` (pierwsze 16 dostępów na port i kierunek).
