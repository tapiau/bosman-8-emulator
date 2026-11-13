# ANG-3001 — sprzęt z punktu widzenia emulatora

Źródło: `../bosman-8-ang-3000-rom` (ROM `ANG3001_ZJ_wersjaPG.BIN`, 16 KB,
dokumentacja w `doc/`, demka w `demo/`).

## CPU i pamięć
- Z80B (zegar nieznany; przyjmujemy 6 MHz, konfigurowalne).
- 0x0000–0x3FFF ROM, 0x4000–0xFFFF RAM; **pod ROM-em też jest RAM** 🟢
  (potwierdzone działaniem w emulatorze): zapis do 0x0000–0x3FFF zawsze
  trafia do RAM-u, **`OUT (0Bh)` wyłącza ROM** (odczyt widzi RAM),
  **`OUT (0Ah)` włącza ROM** (wartość bez znaczenia). Używa tego tylko
  procedura `OUT (0Bh) / LDIR / OUT (0Ah) / RET` kopiowana przy starcie do
  0x7F66 (musi działać z RAM-u), wołana z `ESC (` / `ESC )` (stos okien,
  `sub_0d54h`, `l0db1h`): zawartość ekranu pod otwieranym oknem jest
  odkładana do RAM-u od 0x0000 w górę (deskryptory w 0x7A00+), a przy
  zamykaniu odczytywana przy wyłączonym ROM-ie. W emulatorze `ESC (` +
  `ESC )` przywraca tło bajt w bajt (test `window_push_pop_uses_ram_under_rom`);
  bez przełączania odczyt dałby bajty ROM-u. Po resecie ROM włączony.
- Stos 0x7FA0, zmienne 0x7F00–0x7F9F, gniazda kodu `ESC 0-9` w 0x6000.
- Przerwanie IM1 → pusty `RET` (nieużywane).

## Wideo  🟢
- Bufor 512×256 px, 1 bit/px, **64 B na linię**, od **0x8000** (bank 1,
  0x8000–0xBFFF). Bit 7 bajtu = piksel po lewej.
- Adresowanie kołowe w obrębie banku (`RES 6,H`).
- **Port 0x80 (OUT)** = rejestr scrolla: numer linii pamięci wyświetlanej
  jako pierwsza (8 bit, mod 256).
- Tekst: komórka 6×10 px, 80 kolumn × 25 wierszy; wiersz tekstowy `r` zaczyna
  się w linii pamięci `16 + 10*r + scroll`, kolumna `c` w bicie `2 + 6*c`
  bajtowo od 0x8402 (`VIDEO_ADDR_CALC_TEXT` 0x06C0), tj. x = 16 + 6c px
  (marginesy 16 px z lewej i z prawej).
- **Układ pionowy i offset obrazu** 🟡 (wniosek z kodu, nie pomiar).
  W pamięci, względem rejestru scrolla `s`: linie `s+0..s+9` — wiersz
  komunikatów (BREAK/LOCAL/STOP; ROM czyści 16 linii od `s+0` i pisze tam z
  bazą 0x8002, `sub_1183h`/`sub_0681h`; w adresacji tekstowej to „wiersz 24”,
  bo 16+240 ≡ 0 mod 256), `s+10..s+15` — 6 pustych linii, `s+16..s+255` —
  wiersze robocze 0–23 (`0x7F62..0x7F65` = 0,0,23,79). Przewinięcie o wiersz
  to 10 kroków `s+=1`, w każdym czyszczone są linie `s+15` i `s-1`.
  Wybrany domyślny **`--line-offset 0`**: obraz zaczyna się od linii `s`
  (najprostszy sprzęt: licznik linii ładowany rejestrem scrolla), więc na
  górze ekranu jest pas komunikatów + 6 linii odstępu, a wiersze 0–23
  wypełniają linie ekranu 16–255. Za tym przemawia: projektant adresuje
  pas komunikatów jako „pierwsze 16 linii” (0x8002), tekst jako „od 16-tej”
  (0x8402) — gdyby pas był na dole, naturalny byłby tekst od 0 i komunikaty
  od 240; demka piszące wprost do 0x8000 (SNAKE — ramka planszy, CUBE —
  środek w linii 128) wyglądają poprawnie tylko przy offsecie 0.
  Alternatywa `--line-offset 16` (wiersz 0 w pierwszej linii ekranu, pas
  komunikatów na dole, 6 pustych linii pod nim) też daje spójny obraz przy
  płynnym przewijaniu — rozstrzygnie dopiero zdjęcie ekranu prawdziwego
  terminala (np. w trybie BREAK: czy „BREAK - powtórz” jest na górze).
- Bank 2 (0xC000–0xFFFF) nie jest wyświetlany.

## USART 8251 — porty 0x90 (dane), 0x91 (status/sterowanie)  🟢
Init: `00,00,00,40h` (reset), słowo trybu (bajt ROM 0x0003 = `CDh`, albo
`FAh` gdy bit 5 portu 0x8D = 0), komenda `05h` (TxEN|RxEN).
Status: bit0 TxRDY, bit1 RxRDY, bit2 TxEMPTY (sprawdzany w `CHECK_KEY_BREAK`).
Komenda `0Dh` (bit 3 = send break) używana przy BREAK.
Tryb `CDh` = asynchronicznie ×1, 8 bitów, bez parzystości, 2 bity stopu →
11 bitów/znak; z licznikiem 2 = 13 przy 1,3 MHz daje 100 kbit/s ≈ 9100
znaków/s (emulator liczy czas znaku z tych rejestrów, `--char-cycles`
nadpisuje). Pierwszy bajt odebrany w trakcie startu jest odrzucany
(`IN A,(90h)` tuż przed `MAIN_LOOP`) — emulator wstrzymuje dane z łącza do
osiągnięcia `MAIN_LOOP` (0x04BD).

## Timer 8253 — porty 0x88–0x8B  🟢
Licznik 0 — czas trwania dźwięku (tryb 0, odczytywany do zera w
`WAIT_CTC_DONE` — **musi faktycznie odliczać**, inaczej ROM się zawiesi),
licznik 1 — wysokość tonu (tryb 3), licznik 2 — zegar UART (dzielnik 13).
Zegar wejściowy ~1,3 MHz (szacunek). Licznik 0 prawdopodobnie taktowany
wyjściem licznika 1 (czas trwania w okresach tonu) — tak jest w emulatorze;
dźwięk słychać, gdy licznik 0 (tryb 0) odlicza (OUT=0), częstotliwość =
wyjście licznika 1. Start: pisk 0x0EEF (≈340 Hz, A=5Ah), po teście RAM
0x0D4D, po poprawnej sumie ROM 0x0BDA (≈428 Hz); każdy klawisz to „klik”
(`CTC_LOAD` z A=1, dzielnik 0x08E1), chyba że bit 1 rejestru I.

## Klawiatura  🟢 (potwierdzone na sprzęcie)
- **0x8C IN** — kod klawisza = **ASCII** (tablice 0x0200/0x0280 dotyczą
  tylko trybu z bitem 7 portu 0x8D = 0).
- **0x8E IN** — bit 5: klawisz dostępny (strobe, kasowany odczytem 0x8C);
  bit 2: przełącza się w takt obrazu (50 Hz → ROM dzieli przez 2);
  bit 0: 1 = brak trzymanego klawisza (0 włącza programową autorepetycję).
  W emulatorze bit 2 zmienia stan co 1/50 s czasu CPU (test: 50 zmian/s);
  pętla główna obsługuje klawiaturę co drugą zmianę (25 Hz), więc szybciej
  niż ~25 znaków/s się nie da — emulator trzyma kolejkę klawiszy i podaje
  następny po odczycie 0x8C. Bit 0 domyślnie 1 (autorepetycję robi system
  hosta), API `set_key_held` pozwala użyć autorepetycji ROM-u.
- **0x8D IN** — bit 7: 1 = normalna ścieżka startu; bit 6: 0 = CAPS (odwraca
  wielkość liter); bit 5: 0 = wciśnięty BREAK; bit 4: 🟡 **0 = tryb
  stronicowania**: `CHECK_KEY_BREAK` przy bit 4 = 0 uzbraja licznik linii
  `0x7F0E` (0x9F), a `LF_HANDLER` po pełnym ekranie wypisuje „STOP” w pasie
  komunikatów i czeka na klawisz albo na powrót bitu 4 do 1 (pętla `l0a23h`);
  przy 1 licznik jest zerowany (brak stronicowania). Stan spoczynkowy: `0xF0`.
  Bit 5 = 0 w chwili resetu wybiera tryb 8251 `FAh` i dzielnik z 0x01A6.
- **BREAK** (sprawdzone w emulatorze 🟢): naciśnięcie i puszczenie → w pasie
  komunikatów „BREAK - powtórz   LOCAL - ESC”; ponowny BREAK → komenda 8251
  `0Dh` (nadawanie BREAK ~0,25 s) i powrót; `ESC` → tryb **LOCAL** (napis
  „LOCAL”, klawisze od razu na ekran, nic nie idzie do łącza). Domyślnie po
  starcie tryb DUPLEX: klawisze idą tylko do łącza (bez lokalnego echa).
- Polskie litery: w ROM-owym foncie (glif `c` pod 0x3600 + 10·c, piksele w
  bitach 7..2) kody **0x01 ę, 0x02 ź, 0x03 ł, 0x04 ń, 0x05 ś, 0x06 ą,
  0x07 ó, 0x0B ż, 0x0C ć** 🟢 (odczytane z bitmap); 0x12–0x1C — elementy
  ramek, 0x1F — pełny blok. Stąd bajt 0x07 w „powt\x07rz” to po prostu „ó”
  (`PRINT_STRING` rysuje glify bez interpretacji kodów C0). Odebrane z
  łącza kody 0x01–0x06, 0x0B, 0x0C są rysowane (tabela C0 → `DRAW_CHAR`);
  0x07 to BEL. Jakie kody wysyła klawiatura dla polskich liter — ❓
  (GUI emulatora wysyła kody fontu).

## Sekwencje sterujące — poprawka do wiki
- **`ESC =` / `ESC Y` przyjmują najpierw WIERSZ, potem KOLUMNĘ** (oba +0x20,
  jak w VT52) 🟢 — sprawdzone w emulatorze. Wiki ROM-u podaje odwrotnie:
  stan 7 maszyny parsera kopiuje pierwszy bajt z 0x7F06 do 0x7F07, a
  `CURSOR_POS_SET` bierze (0x7F06) = drugi bajt → IXH (kolumna) i
  (0x7F07) = pierwszy → IXL (wiersz).
- `ESC ( w1 k1 w2 k2` — otwarcie okna (zapamiętuje tło, ogranicza obszar
  pracy), `ESC )` — zamknięcie i odtworzenie tła 🟢 (emulator).
- `ESC H`, `ESC J`, `ESC K`, CR/LF, przewijanie przez port 0x80 — działają
  zgodnie z wiki.

## Inne porty
0x00/0x01, 0x8F (`BBh` przy starcie) — init wideo, ignorowane.
0x08/0x09 — strobe'y przy `ESC ^`/`ESC \`, ignorowane.
Emulator z `--trace-io` loguje dostęp do wszystkich nieobsługiwanych portów
oraz 0x0A/0x0B. Przy zwykłej pracy (start, tekst, przewijanie, BREAK,
okna) innych portów ROM nie używa.
