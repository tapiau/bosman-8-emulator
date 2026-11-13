# ANG-3001 — sprzęt z punktu widzenia emulatora

Źródło: `../bosman-8-ang-3000-rom` (ROM `ANG3001_ZJ_wersjaPG.BIN`, 16 KB,
dokumentacja w `doc/`, demka w `demo/`).

## CPU i pamięć
- Z80B (zegar nieznany; przyjmujemy 6 MHz, konfigurowalne).
- 0x0000–0x3FFF ROM, 0x4000–0xFFFF RAM. (Pod ROM-em prawdopodobnie też jest
  RAM — porty 0x0A/0x0B otaczają `LDIR` w procedurze kopiowanej do 0x7F66;
  hipoteza: zatrzask jak w Bosmanie, Q5 = RAM pod ROM-em. Na razie logować.)
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
  bajtowo od 0x8402 (`VIDEO_ADDR_CALC_TEXT` 0x06C0). ❓ czy sprzęt dodaje
  stały offset do rejestru scrolla — sprawdzić empirycznie (domyślnie
  wyświetlamy od linii `scroll`, offset konfigurowalny).
- Bank 2 (0xC000–0xFFFF) nie jest wyświetlany.

## USART 8251 — porty 0x90 (dane), 0x91 (status/sterowanie)  🟢
Init: `00,00,00,40h` (reset), słowo trybu (bajt ROM 0x0003 = `CDh`, albo
`FAh` gdy bit 5 portu 0x8D = 0), komenda `05h` (TxEN|RxEN).
Status: bit0 TxRDY, bit1 RxRDY, bit2 TxEMPTY (sprawdzany w `CHECK_KEY_BREAK`).
Komenda `0Dh` (bit 3 = send break) używana przy BREAK.

## Timer 8253 — porty 0x88–0x8B  🟢
Licznik 0 — czas trwania dźwięku (tryb 0, odczytywany do zera w
`WAIT_CTC_DONE` — **musi faktycznie odliczać**, inaczej ROM się zawiesi),
licznik 1 — wysokość tonu (tryb 3), licznik 2 — zegar UART (dzielnik 13).
Zegar wejściowy ~1,3 MHz (szacunek). Licznik 0 prawdopodobnie taktowany
wyjściem licznika 1 (czas trwania w okresach tonu).

## Klawiatura  🟢 (potwierdzone na sprzęcie)
- **0x8C IN** — kod klawisza = **ASCII** (tablice 0x0200/0x0280 dotyczą
  tylko trybu z bitem 7 portu 0x8D = 0).
- **0x8E IN** — bit 5: klawisz dostępny (strobe, kasowany odczytem 0x8C);
  bit 2: przełącza się w takt obrazu (50 Hz → ROM dzieli przez 2);
  bit 0: 1 = brak trzymanego klawisza (0 włącza programową autorepetycję).
- **0x8D IN** — bit 7: 1 = normalna ścieżka startu; bit 6: 0 = CAPS (odwraca
  wielkość liter); bit 5: 0 = wciśnięty BREAK; bit 4: ❓ (autorepetycja).
  Stan spoczynkowy: `0xF0`.

## Inne porty
0x00/0x01, 0x8F (`BBh` przy starcie) — init wideo, ignorowane.
0x08/0x09 — strobe'y przy `ESC ^`/`ESC \`, ignorowane.
