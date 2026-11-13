# Architektura emulatora Bosman-8 + ANG-3001

## Cele

1. Dwa **niezależne** emulatory (osobne procesy / binarki):
   - `bosman8` — komputer Bosman-8 (DZM-180), Z80A 4 MHz, CPM-R w EPROM 32 KB.
   - `ang3001` — terminal graficzny ANG-3001, Z80B, EPROM 16 KB, ekran 512×256.
2. Łączy je **wirtualne łącze szeregowe** — strumień bajtów, domyślnie TCP
   (Bosman nasłuchuje, terminal się łączy).
3. Każdy kanał szeregowy emulowanej maszyny można przepiąć na **port szeregowy
   hosta** (`/dev/ttyUSB0`, `COM3`, …), na stdio albo zostawić niepodłączony.
4. Linux / Windows / macOS natywnie; w przyszłości wersja webowa (WASM).

## Język i struktura

Rust (workspace). Rdzenie emulatorów są czystą logiką bez wątków, okien i
socketów — dzięki temu skompilują się do WASM bez zmian. Wszystko, co zależy
od systemu (okno, gniazda, port szeregowy, zegar), siedzi w aplikacjach.

```
crates/
  b8-link/      Trait SerialEndpoint + backendy: TCP (listen/connect),
                port szeregowy hosta (serialport), stdio, null, para w pamięci.
                Backendy z I/O działają w wątku tła i komunikują się kanałami,
                więc emulator nigdy nie blokuje się na I/O.
  b8-chips/     Wspólne układy: zatrzask 74LS259, 8253 (PIT), Z80-SIO,
                8251 USART, WD1770. Bez zależności od systemu.
  bosman8/      Rdzeń Bosmana (pamięć + bankowanie, porty) + binarka CLI.
  ang3001/      Rdzeń terminala (pamięć, wideo, klawiatura, dźwięk) + binarka GUI.
```

CPU: crate [`iz80`](https://crates.io/crates/iz80) (Z80, przechodzi ZEXALL,
liczy cykle). Brak IM0/IM2 nie przeszkadza — obie maszyny używają tylko IM1/NMI.

## Model łącza szeregowego

Na poziomie emulacji łącze to **strumień bajtów** (bez symulacji bitów, zegara
synchronicznego itp.). Każdy kanał emulowanego układu (SIO-A, SIO-B, 8251):

- przy zapisie do rejestru danych wysyła bajt do endpointu i ustawia „TX busy”
  na czas jednego znaku (liczony z zegara 8253 i dzielnika układu),
- bajty przychodzące trafiają do kolejki po stronie hosta; do rejestru
  odbiorczego wpada następny bajt dopiero gdy poprzedni został odczytany i minął
  czas znaku — dzięki temu nie ma przepełnień, których na prawdziwym sprzęcie
  pilnowałby protokół.

Oryginalnie Bosman ↔ ANG pracują **synchronicznie, ~100 kbit/s** — zwykły UART
PC tego nie zrobi, więc przy podpinaniu prawdziwego terminala/komputera przez
port hosta potrzebny będzie adapter albo przestawienie trybu w emulowanym
systemie (dla SIO-B Bosmana — async 9600 — działa wprost).

## Czas

Każdy rdzeń wystawia `run_cycles(n)`; aplikacja dzieli czas rzeczywisty na
plasterki (1/50 s), wykonuje odpowiednią liczbę cykli i śpi resztę. Opcja
`--turbo` wyłącza synchronizację z zegarem.
