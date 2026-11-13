# Bosman-8 — „sieć” po V.24: napędy D/E/F i tryb DEBUG (slave)

Ustalone z kodu ROM i sprawdzone w emulatorze (2026-11-13, T4.4). Poprawia
`bosman-8-rom/doc/wiki/disk_interlink.md`, który twierdzi, że strony serwera
nie ma w ROM-ie — **jest** (0x1564…0x17E8).

Pewność: 🟢 potwierdzone (kod jednoznaczny + działa między dwoma emulatorami),
🟡 prawdopodobne (kod przeczytany, nieużywany przez mastera, nie testowane),
❓ hipoteza.

## Zgodność ROM-ów — przyczyna problemu z prawdziwym Bosmanem

Opis protokołu 60/F0–F3 poniżej dotyczy ROM-u **SAJ 32 KB** używanego
w emulatorze. Oba obrazy **UNIMOR ZJ 16 KB** z repozytorium `bosman-8-rom`
mają starszy monitor DEBUG bez rozkazu 60:

| ROM | Pętla monitora | Weryfikacja rozkazu | Rozkaz 60 |
|---|---|---|---|
| ZJ CPMR 1987-03-24 | 0ED0 | 0ED7: `AND F1`; `CP 51`; `CP 50`; odrzucenie innych | ignorowany |
| ZJCPMR 2311 1987-10-20 | 0F66 | 0F6D: taka sama weryfikacja | ignorowany |
| SAJ / ANG-3001 | 1597 | zakres 50–60, tablica 15E3 | serwer dysków 1724 |

W ZJ maska dopuszcza wyłącznie 50–5F. Dla 60 wynik AND F1 to 60,
więc monitor wraca do odbioru; nie pobiera nawet parametru 00. Nie zmienia
licznika 8253 i nie interpretuje późniejszego F1 jako nagłówka dyskowego.

**Podłączony Bosman to wersja 1987-10-20**: 32 bajty od F4A4 odczytane
przez DEBUG odpowiadają dokładnie bajtom ROM-u ZJCPMR 2311 od 13A4
(obraz BIOS kopiowany z 1100 do F200). Wersja marcowa i SAJ mają w tym
miejscu inne bajty. Powitanie 80 88 i niezmieniony F363=0D po 60 są
zgodne z kodem październikowego monitora.

Zatem brak odpowiedzi po 60 w tej konfiguracji wynika z **niezgodności
protokołu ROM-ów**. Zmiany timingu TX ani wybór 19200/20833 nie dodadzą
brakującego serwera. Obsługa D/E/F wymaga zgodnego serwera (np. programu
w RAM dostosowanego do BIOS-u ZJ) albo ROM-u obsługującego protokół SAJ.
Gotowy program [BSRV.COM](../tools/bsrv/README.md) uruchamia taki serwer
w RAM-ie. Testy z oboma ROM-ami ZJ obejmują katalog, PIP w obie strony
i zwolnienie łącza. Uploader przez DEBUG instaluje go na B: wersji
październikowej z odczytem kontrolnym.

## Idea

Dwa Bosmany łączy się kablem między kanałami **SIO-B** (V.24). Jeden przechodzi
w **tryb podporządkowany** (slave, „mikrosystem”) — zdalny monitor sterowany
bajtowymi rozkazami 50h–60h. Drugi (master) widzi wtedy napędy slave'a jako
**D:, E:, F: = A:, B:, C: slave'a** 🟢. Master najpierw odczytuje z pamięci
slave'a jego tablice (DPB, maska napędów), potem rozkazem 60h przełącza go w
pętlę „serwera dysków” (rozkazy F0h–F3h), która woła BIOS slave'a
(HOME/SELDSK/SETTRK/SETSEC/READ/WRITE) na sektorach 128-bajtowych.

## Jak tego użyć w emulatorze  🟢

Dwa procesy połączone przez TCP (serwer = slave, klient = master):

```sh
cp disks/narzedzia.img /tmp/serwer.img
# serwer (slave): jego B: będzie u klienta jako E:
bosman8 --term stdio --aux tcp-listen:127.0.0.1:4000 --disk-b /tmp/serwer.img
#   A>DEBUG          ← polecenie CCP; ekran milknie, Bosman czeka na rozkazy
# klient (master)
bosman8 --term stdio --aux tcp:127.0.0.1:4000
#   A>DIR E:
#   Odwołanie do dysków D E F - dołącz (D)  rezygnuj (R)  ?   D
#   E: PIP      COM : STAT     COM : ED       COM : SUBMIT   COM
#   ...
#   A>E:PIP A:=E:STAT.COM      ← odczyt (PIP też ładuje się z E:)
#   A>STAT
#   A: R/W, Space: 398k
#   A>E:PIP E:KOPIA.COM=A:STAT.COM   ← zapis na dyskietkę serwera
```

- Zamiast `--term stdio` może być terminal ANG (`--term tcp-listen:…` + `ang3001`).
- **Nie używaj `--turbo`** po żadnej stronie: master ma programowe timeouty
  (~0,1 s na bajt), a obie maszyny muszą iść w tym samym tempie czasu
  emulowanego (czas rzeczywisty).
- Zwolnienie łącza: odwołanie do napędu ≥ G: (np. `DIR G:`) → master wysyła
  F0h, slave wraca do CCP (`A>`). Ponowne odwołanie do D/E/F na nowo zadaje
  pytanie D/R — slave musi być wtedy znów w `DEBUG`.
- Podgląd łącza: `--trace-aux` (na stderr, z czasem emulowanym w s):
  `aux    9.043549 -> 56` (wysłane), `aux   10.061065 <- 56 AB 58 F2 5B`
  (odebrane z linii w jednej porcji), `<- (Rx disabled, lost)` (bajty, które
  przyszły, zanim ROM włączył odbiornik SIO-B — giną jak na prawdziwym SIO).
- Test bez sieci: `crates/bosman8/tests/interlink.rs` — dwa rdzenie w jednym
  procesie połączone `b8_link::pair()`, kwanty po 20 ms jak w binarce
  (~5 s w buildzie testowym; `TRACE_AUX=1` wypisuje ruch).

### Z prawdziwym Bosmanem  🟡 (nie testowane na sprzęcie)

Kabel V.24 (TxD/RxD skrzyżowane, masa) z SIO-B Bosmana do adaptera USB-RS232:

```sh
bosman8 --term stdio --aux serial:/dev/ttyUSB0@9600   # emulator = master, prawdziwy = DEBUG
bosman8 --term stdio --aux serial:/dev/ttyUSB0@9600 --disk-b dysk.img   # albo odwrotnie
```

Uwaga: łącze **zaczyna na 9600 bit/s, a po rozkazie 60h obie strony
przełączają się na szybszą prędkość**: 8253 licznik 2 = 13 → 6 (bajty
`F363/F364` konfiguracji V.24 `F360..F364`, używanej po 60h przez obie strony); przy zegarze 2 MHz i
×16 daje to 9615 → **20833 bit/s** (nominalnie pewnie „19200”, ale z błędem
+8,5% — poza tolerancją zwykłego UART-u ustawionego na 19200). Emulator
przestawia port hosta razem z emulowanym SIO-B (`Endpoint::set_line_params`,
zob. bosman8-hardware.md): 9615 → 20 833 po rozkazie 60h i z powrotem po F0h
(test `interlink.rs` sprawdza tę sekwencję). Adapter musi przyjmować
niestandardowe prędkości — PL2303HX ustawia 20 833 bit/s dokładnie
(sprawdzone), FTDI i CP210x zwykle też. 🟡 Transfer z prawdziwym Bosmanem
jeszcze nie był próbowany. Między dwoma emulatorami (TCP, `pair`) prędkość
linii nie ma znaczenia.

## Wejście w tryb slave  🟢

- Polecenie CCP **`DEBUG`** (tablica poleceń CCP 0x4432: `DIR 446C, ERA 451A,
  TYPE 4568, SAVE 45A4, REN 45F5, USER 4662, DEBUG 3F11, D 467A`). Handler
  0x3F11 ustawia bit 0 `(IX-54h)`, ładuje program podany w reszcie linii (do
  zdalnego debugowania) i skacze do **0x1564**.
- „H-ERROR - mikrosystem (M) ignoruj (I)” przy starcie (brak odpowiedzi
  kontrolera dyskietek) — M → `JP 1564h` (0x0E2E): Bosman bez stacji może być
  „mikrosystemem” sterowanym z drugiego komputera.

```
1564  DI ; SP=FBC0 ; SIOB_INIT(F36A) (9600 bit/s) ; F35F=0, FBF8=0
      zeruje FBC0..FBD1 (zapisane rejestry), kopiuje 1539..1560 → FBD2 (stub
      „odtwórz rejestry i skocz”), F4CD := JP 15BA (pułapka RST 30h → 15BA)
158D  wyślij C, 88h      ; „gotów”: przy wejściu C=00 → 00 88; po 5Dh C=01 → 01 88
                         ; (ROM ładuje 80h do A, ale procedura 1276 wysyła C)
1597  pętla: b = odbierz; b-50h ∈ 0..10h → skok przez tablicę 15E3, inne ignoruje;
      rozkazy parzyste (50,52,…,60) odbierają 1 bajt parametru (→ C)
```

## Rozkazy slave'a (tablica 0x15E3)

Rejestry adresowe slave'a: **A1 = FBFA** (bieżący), **A2 = FBFC**, **A3 = FBFE**.
Rozkazy 56h/58h działają jak stos: nowy bajt wchodzi do A1, stary A1 → A2,
stary A2 → A3 (osobno młodsze i starsze bajty). Odczyty pamięci idą przez
`F452` (gdy `FBF8`=0 — zawsze w trybie slave — włącza RAM zamiast ROM-u w
0000–7FFF). „Suma” = suma bajtów mod 256.

| Rozkaz | Adres | Działanie | Odpowiedź | Pewność |
|---|---|---|---|---|
| 50 lo | 1605 | `50 lo hi nlo nhi dane×n`: zapis bloku pod adres hi:lo | suma (od `lo` do końca danych) | 🟡 |
| 51 | 1614 | porównanie [A3..A2] z blokiem od A1 | 2 B: pozostały licznik (0000 = zgodne; FFFF gdy A2<A3) | 🟡 |
| 52 lo | 1638 | `52 lo hi nlo nhi`: odczyt bloku | n bajtów + suma (od `lo`) | 🟡 |
| 53 | 164E | starszy bajt sumy 16-bit z ostatniego 57h (`FBF4`) | 1 B | 🟡 |
| 54 x | 1653 | wypełnienie [A2..A1] bajtem x | 55h | 🟡 |
| 55 | 1662 | identyfikacja: bajt spod `F4A4` (80h), A1 := F4A5 (dalej `60 FB "BOSMAN-8" 00`, czytać 5Bh) | 1 B | 🟡 |
| 56 x | 1667 | A1.lo := x (przesuwa A1→A2→A3) | — | 🟢 |
| 57 | 166C | suma 16-bit [A2..A1] | młodszy bajt (starszy → 53h) | 🟡 |
| 58 x | 1687 | A1.hi := x (przesuwa) | — | 🟢 |
| 59 | 1694 | GO: odtwarza rejestry z FBC0.. i skacze (`F4BE`: RAM, `JP FBD4`) | po pułapce 81h (zob. 15BA) | 🟡 |
| 5A x | 1698 | (A1) := x, A1++ | — | 🟡 |
| 5B | 16A1 | wyślij (A1), A1++ | 1 B | 🟢 |
| 5C x | 16AF | CALL A1 z A=x (RAM włączony, `F4B0`) | A po powrocie | 🟡 |
| 5D | 158D | „hello” | `01 88` | 🟢 |
| 5E | 03DA | koniec trybu slave, gorący start → CCP | — | 🟡 |
| 5F | 16BD | przesunięcie bloku [A3..A2] → A1 (z obsługą nakładania) | 55h | 🟡 |
| 60 x | 1724 | przejście w **serwer dysków** (x ignorowany), patrz niżej | — | 🟢 |
| — | 15BA | wejście z pułapki/breakpointu (`RST 30h` → F4C3): zapis rejestrów do FBC0.., wysłanie 81h, powrót do pętli | 81h | 🟡 |

## Serwer dysków (0x1724)  🟢

Wejście: `F363`=06 (8253 licznik 2 = 6 → **19200 bit/s**), `F35F`=0 (następne
odebranie ponownie inicjuje SIO-B), DMA := F100. Pętla: odbierz bajt; < F0h —
ignoruj.

| Rozkaz | Bajty (master → slave) | Działanie slave'a | Odpowiedź |
|---|---|---|---|
| F0 | `F0 xx` | `F363`=0D (9600), odbiera xx, gorący start → CCP | — |
| F1 | `F1 h t s d k` | h≠0 → BIOS HOME; SETTRK(t); SETSEC(s); SELDSK(d) (d = 0..2 = A/B/C slave'a); k = −(F1+h+t+s+d) | suma kontrolna (00 = OK) albo 5Ah, gdy SELDSK zwróci 0 |
| F2 | `F2 k` (k = −F2 = 0Eh) | potwierdza, potem BIOS READ do F100 | `00`, potem 128 B + (−suma) |
| F3 | `F3 w dane×128 k` (k = −suma od F3) | gdy suma OK: BIOS WRITE(C=w) z F100 | 00 = zapisane; ≠0 = zła suma |

Master po złej sumie powtarza (nagłówek F1 albo cały odczyt/zapis). Sektory są
logiczne, 128-bajtowe — deblokowanie robi BIOS slave'a, więc działa każdy
napęd slave'a, w tym RAM-dysk.

## Master — dostęp do D/E/F (SELDSK 0x0476)  🟢

- C (napęd) ≥ 6 → **zwolnienie** (0x0563): wyślij F0h, `F363`=0D, 3 ms, wyślij
  00, potem jak błąd: `F268 &= 3Fh`, `F370 &= C7h`, `F267`=1Ah, wynik 0.
- C < 3 → zwykły SELDSK lokalny. C = 3..5 przy `F267`≠0 (rozłączony; wartość
  początkowa 1Ah z obrazu BIOS-u) → **połączenie**:
  1. pytanie „Odwołanie do dysków D E F - dołącz (D) rezygnuj (R)” (R → wynik 0),
     `F268 |= C0h`;
  2. `F35F`=0, `F363`=0D (9600), inicjalizacja SIO-B (`SIOB_INIT` 0x1487), `F267`=0,
     timeout `FA19`=1000h;
  3. wysyła **5D** i czyta bajty aż do **88h**;
  4. `56 AB 58 F2`, 60× `5B` → DPB slave'a A/B/C (`F2AB`, po 20 B) do lokalnego
     `F9A0`/`F9B4`/`F9C8`;
  5. `56 70 58 F3`, `5B` → maska napędów slave'a `F370`; `F370 |= maska<<3`
     (bity 3–5 = D/E/F);
  6. gdy był timeout (`F267`≠0) → „Brak komunikacji z drugim komputerem”;
  7. wysyła **60 00**, ustawia `F363`=06 (19200), `F35F`=0.
- Wybór: bit C maski `F370` (brak → „brak napędu”), DPH z tablicy `F4A0+16·C`
  (`F4D0`/`F4E0`/`F4F0`, DPB → `F9A0`/`F9B4`/`F9C8`).
- READ (0x064B): nagłówek F1 (0x06AA: `F438` = flaga HOME, `F34E` ścieżka,
  `F350` sektor, `F34D`−3 napęd), potem F2 i 128 B do DMA `F351` + suma.
  WRITE (0x067C): nagłówek F1, potem F3. HOME (0x06D4) tylko ustawia `F438`.
- Wysyłanie: 0x0637 (`C_PUNCH` 0x1247 → 0x1276: czeka na RR1 „all sent”, `OUT 81h`;
  nic nie wysyła, gdy `F267`≠0); odbiór: 0x060C (odpytuje 0x1349, licznik `FA19`
  — 1000h ≈ 0,1 s, FF00h ≈ 7 s przy blokach/zapisie; po timeoucie `F267`=1Ah).
- Odpowiedzi po timeoucie są traktowane jak OK (dane śmieciowe), ale `F267`≠0
  wymusza ponowne połączenie przy następnym SELDSK.

## Co było nie tak w emulatorze (T4.4)  🟢

1. **Utracony bajt po przełączeniu na 19200.** `SIOB_INIT` (0x1487) programuje
   licznik 8253, czyta rejestr danych, czeka 2 ms i czyta go znowu — co przyjdzie
   w tym oknie, ginie. Na prawdziwej linii pierwszy bajt mastera po rozkazie
   60h (F1) dociera ~0,4 ms *po* drugim odczycie slave'a (master też robi 2 ms
   opóźnienia po zmianie prędkości). W emulatorze bajty z TCP (i z kwantów
   20 ms obu emulatorów) przychodzą paczkami: `60 00 F1` w jednej porcji, więc
   F1 trafiał do odbiornika w trakcie okna 2 ms i był wyrzucany; slave czekał na
   F1, master na odpowiedź → timeout, śmieciowe dane, „Brak pliku”.
   Naprawa: po zapisie licznika 8253 taktującego kanał (port 85h → SIO-A,
   86h → SIO-B) odbiornik przez 3 ms nie pobiera bajtów z kolejki hosta
   (`SioChannel::hold_rx`, `RX_RESYNC_CYCLES` w `crates/bosman8/src/lib.rs`).
2. **3-bajtowa kolejka odbiorcza SIO** (`b8-chips`): wcześniej bajty czekały w
   nieskończonej kolejce hosta za 1-bajtowym rejestrem, więc „stare” bajty
   (np. powitanie `00 88` slave'a sprzed połączenia) przeżywały odczyty
   czyszczące `SIOB_INIT` i przesuwały protokół o 2 bajty („E: brak napędu”).
   Teraz jak w Z80-SIO: do 3 bajtów w FIFO układu (czyszczone przez odczyty),
   reszta czeka w kolejce hosta (bez przepełnień — wygoda dla terminala).
3. **SIO-B gubi bajty przy wyłączonym odbiorniku** (WR3 bit 0 = 0), jak
   prawdziwy układ — wcześniej czekały do włączenia. Kanał A (terminal) nadal je
   zachowuje.

Bez poprawki 1 test `interlink` daje dokładnie objaw z ręcznej próby
(„Brak pliku”).

## Otwarte

- Próby z prawdziwym Bosmanem: handshake i odczyt DPB przy 9615 bit/s działają;
  po `60 00` i zmianie na 20833 bit/s brak odpowiedzi na nagłówek F1
  (stan z commita `e90c37a`). Samo ustawianie prędkości na adapterze nie
  potwierdza działania transferu.
- Kolejna poprawka do sprawdzenia na sprzęcie: zmiana parametrów portu ma
  teraz znacznik czasu emulowanego (`set_line_params_at`). Po opróżnieniu TX
  i konfiguracji portu backend ustawia nową bazę czasu, zachowując odstęp
  do kolejnego bajtu. Wcześniej opóźnienie sterownika USB mogło zużyć całą
  przerwę między zmianą prędkości a F1. Test zegara TX obejmuje konfigurację
  trwającą 20 ms i zachowanie 2,5 ms przerwy. To hipoteza przyczyny awarii,
  wymagająca ponownego testu z prawdziwym Bosmanem.
- Rozkazy 50–5F poza 56/58/5B/5D nie są używane przez mastera D/E/F — opis z
  kodu, nietestowany (zdalny debugger — klient po stronie PC nie istnieje).
- Wyświetlanie: w trybie slave ekran serwera stoi (brak komunikatów) — tak ma ROM.

### Ponowna próba po poprawce zegara TX

Po ponownym podłączeniu PL2303 do `/dev/ttyUSB0` próba `DIR D:` nadal
kończy się brakiem odpowiedzi po zmianie prędkości. Odczyt DPB i maski
napędów działa; powitanie prawdziwego Bosmana to `80 88`. Sama poprawka
zachowania przerwy nie rozwiązała problemu. Następny krok: porównać
konfigurację V.24 w RAM prawdziwej maszyny z emulatorem, po ponownym wejściu
w `DEBUG`. Narzędzie `cargo run -p b8-link --example interlink_inspect --
/dev/ttyUSB0` odczytuje F360, F2AB i F4A4 przez rozkazy monitora; nie
przełącza do serwera dysków i nie zapisuje pamięci. Odczyty monitora poniżej
8000h pokazują RAM pod ROM-em, więc nie nadają się do zrzutu kodu ROM.

Odczyt z prawdziwego Bosmana po ponownym `DEBUG`: `F360..F36E` =
`E1 4C EA 0D 00 E1 4C EA 0D 00 E1 4C EA 0D 00`. Pod F4A4 nie ma
identyfikacji opisanej dla ROM-u emulatora, lecz kod (`C0 D3 0E C9 ...`),
a hello wynosi `80 88`: nie należy zakładać identycznego układu ROM/BIOS.
Osobna próba nagłówka F1 przy 20833 bit/s, z przerwą 50 ms przed i po
zmianie prędkości oraz 3 ms między bajtami, również nie dała odpowiedzi.
Skrócenie przerwy przez emulator nie wyjaśnia więc całego problemu.
Opcjonalny drugi argument `interlink_inspect DEVICE DISK_BAUD` uruchamia
rozkaz 60, wysyła wyłącznie nagłówek F1 (HOME/wybór napędu A), sprawdza
odpowiedź i próbuje zwolnić serwer rozkazem F0; nie czyta ani nie zapisuje
sektorów. Po nieudanej próbie może być konieczny reset i ponowne `DEBUG`.

### Rozkaz 60 nie przełącza badanego Bosmana

Dalsza próba przy 19200 bit/s również bez odpowiedzi na F1. Następnie,
**bez F0 i bez resetu**, monitor ponownie odpowiedział `80 88` przy 9615
bit/s; odczyt F360 nadal pokazał licznik F363 = 0D. Zatem badany Bosman
pozostaje w monitorze po `60 00`, zamiast przejść do serwera dysków jak
ROM emulatora. Różna implementacja/obsługa 60 w rzeczywistym ROM-ie jest
obecnie głównym tropem; sam brak odpowiedzi nie potwierdza awarii zmiany
prędkości. Potrzebny zrzut rzeczywistego ROM-u do porównania procedury
monitora i jego tablicy rozkazów. Tryb `--active` narzędzia pozwala wysłać
sam nagłówek przy zadanej prędkości, pomijając powitanie i rozkaz 60.
Po timeoucie narzędzie nie wysyła F0.

### BSRV na sprzęcie — przełączanie portu USB

Po instalacji i uruchomieniu BSRV.COM powitanie `00 88` oraz odczyt DPB
przez emulator działają, lecz pierwsza próba `DIR E:` nie otrzymała
odpowiedzi po zmianie prędkości. BSRV nadal odpowiadał przy 9615 bit/s.
Osobna próba `60 00`, 50 ms przerwy przed ustawieniem 20833 bit/s i
50 ms po nim dała poprawną odpowiedź `00` na F1. Potwierdza to działanie
serwera przy szybszej prędkości i wskazuje na odstęp przed przestawieniem
adaptera USB. Backend dodaje teraz 20 ms ochrony po ostatnim zapisie TX
przed zmianą parametrów portu, oprócz zachowania przerwy po konfiguracji.
Pełny test emulatora z tą poprawką po ponownym uruchomieniu BSRV **przeszedł
na sprzęcie**: katalog E:, odczyt BSRV.COM, zapis pliku tymczasowego,
odczyt kontrolny (640 bajtów zgodnych), usunięcie pliku i zwolnienie F0.
Szczegóły i SHA-256 w [tools/bsrv/README.md](../tools/bsrv/README.md).
