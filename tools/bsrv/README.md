# BSRV.COM — serwer dysków dla Bosmana UNIMOR ZJ

Program CP/M dla ROM-ów ZJ 1987-03-24 i ZJCPMR 2311 1987-10-20.
Udostępnia A:/B:/C: klientowi z ROM-em SAJ jako D:/E:/F: po SIO-B.
Nie wymaga zmiany EPROM-u. Binarne obrazy ZJ w `roms/bosman8/` są kopiami plików
użytkownika z sąsiedniego repozytorium `bosman-8-rom`, użytymi w testach.

## Uruchomienie

Na prawdziwym Bosmanie (user 0):

```
A>B:BSRV
```

Na komputerze hosta, emulator jako klient:

```sh
cargo run -p bosman8 -- --no-gui --term stdio \
  --aux serial:/dev/ttyUSB0@9600 --trace-aux
```

W emulatorze `DIR E:` i odpowiedź `D` na pytanie o połączenie.
`E:` to fizyczny napęd B: Bosmana. `DIR G:` kończy sesję i przywraca
CCP na serwerze. Serwer trzeba wtedy uruchomić ponownie przez `B:BSRV`.
Obie strony pracują w czasie rzeczywistym (bez `--turbo`).

Serwer zaczyna przy 9615 bit/s, 8N2; po rozkazie 60 przechodzi na
20833 bit/s. RTS i DTR są aktywne. Obsługuje rozkazy inicjalizacji
56/58/5B/5D oraz 60, potem F0/F1/F2/F3. Nie zastępuje pełnego DEBUG.
Dane odczytuje/zapisuje przez standardowe wektory BIOS F218–F22A.
Kod przenosi się do D000, stos D800, bufor DMA E800; TPA nie może
być jednocześnie używane przez inne programy. F3 rzeczywiście zapisuje
na udostępnionym dysku — to pełny serwer odczytu/zapisu.

## Kompilacja i testy

Wymagany `z80asm`:

```sh
tools/bsrv/build.sh
cargo test -p bosman8 --no-default-features --test interlink --test bsrv_install
cargo build -p b8-link --example install_bsrv
```

COM ma maksymalnie pięć rekordów CP/M. Instalator zapisuje go z dopełnieniem
do 640 bajtów. Po zmianie źródeł uruchom build.sh i przebuduj uploader:
pliki binarne są do niego włączone przy kompilacji.

## Instalacja przez DEBUG

Prawdziwy Bosman: dyskietka w B:, następnie `DEBUG`.

```sh
cargo run -p b8-link --example install_bsrv -- /dev/ttyUSB0
```

Uploader sprawdza sygnaturę BIOS-u **1987-10-20**, przesyła instalator
pod D800 i COM pod E000, sprawdza każdy przesłany bajt przez odczyt DEBUG,
potem uruchamia instalator rozkazem 59. Instalator używa BDOS do utworzenia
`B:BSRV.COM` w user 0, odmawia nadpisania istniejącego pliku, zapisuje pięć
rekordów, zamyka i otwiera plik ponownie oraz porównuje wszystkie bajty.
Odpowiedź `B8I:K` oznacza sukces; `E` = plik istnieje, `C` = create/close,
`W` = write, `O` = ponowne otwarcie, `R` = read, `V` = różnica danych.
Po błędzie może pozostać częściowy plik; nie jest automatycznie usuwany.
Po zakończeniu Bosman wraca do CCP. Automatyczny uploader obsługuje obecnie
wyłącznie ROM październikowy; sam BSRV.COM działa też z marcowym.

Test instalatora wykonuje ten sam transfer przez DEBUG i start przez 59
w emulatorze z ROM-em październikowym, sprawdza zawartość pliku oraz odmowę
ponownej instalacji. Testy interlink uruchamiają COM z dyskietki w obu ROM-ach
ZJ i sprawdzają współpracę z klientem SAJ: katalog, PIP w obie strony i wyjście.

## Potwierdzona instalacja na sprzęcie

2026-11-13: PL2303 `/dev/ttyUSB0`, Bosman ZJCPMR 2311 1987-10-20.
Wszystkie fragmenty RAM-u zweryfikowane przez DEBUG, następnie `B8I:K`:
`B:BSRV.COM` w user 0, 640 bajtów, zamknięty i ponownie odczytany bez różnic.
Po dodaniu 20 ms ochrony TX przed zmianą parametrów adaptera USB pełny
transfer z klientem SAJ został potwierdzony na prawdziwym Bosmanie:

- `DIR E:` pokazał katalog fizycznego B:, w tym BSRV.COM.
- `B:PIP B:=E:BSRV.COM`: odczyt 640 bajtów, pełna zgodność z instalowanym COM.
- `B:PIP E:B8TD504A.TMP=B:BSRV.COM`: zapis pliku testowego na fizyczne B:.
- `B:PIP B:RETURN.COM=E:B8TD504A.TMP`: odczyt kontrolny, wszystkie bajty zgodne.
- `ERA E:B8TD504A.TMP`, następnie `DIR E:B8TD504A.TMP` → `Brak pliku`.
- `DIR G:` wysłał zwolnienie serwera F0.

SHA-256 obu odczytanych plików (640 bajtów):
`5fbfd5525d1f33b60532061e01bc7954248d3e7e51c67b8480d870dc86424831`.
Ślady próby: `/tmp/bsrv-hardware.err`, `/tmp/bsrv-hardware.out`;
porównane pliki `/tmp/bsrv-from-hardware.com`, `/tmp/bsrv-write-readback.com`.
Test sprzętowy dotyczy napędu E: klienta / B: serwera; A: i C: serwera
objęte są testami emulowanymi w zakresie opisanym wyżej.
