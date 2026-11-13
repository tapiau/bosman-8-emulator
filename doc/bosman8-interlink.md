# Bosman-8 — „sieć” po V.24: napędy D/E/F i tryb DEBUG (slave)

Ustalone z kodu ROM (2026-11-13). Poprawia `bosman-8-rom/doc/wiki/disk_interlink.md`,
który twierdzi, że strona serwera nie jest w ROM-ie — **jest**.

## Idea

Dwa Bosmany łączy się kablem między kanałami **SIO-B** (V.24, domyślnie
~9600 bit/s async). Jeden komputer przechodzi w **tryb podporządkowany**
(slave, „mikrosystem”) — to w praktyce zdalny monitor/debugger sterowany
bajtowymi rozkazami. Drugi (master) widzi wtedy napędy slave'a jako swoje
**D:, E:, F:**. Master nie wysyła „rozkazów dyskowych” — czyta tablice BIOS-u
slave'a z jego pamięci i zdalnie wykonuje jego procedury BIOS (odczyt pamięci,
zapis pamięci, wywołanie kodu). Ten sam mechanizm służy do zdalnego
debugowania programu (`DEBUG PROG`).

## Jak wejść w tryb slave  🟢

- Polecenie CCP **`DEBUG`** (wbudowane; tablica poleceń CCP 0x4432:
  `DIR 446C, ERA 451A, TYPE 4568, SAVE 45A4, REN 45F5, USER 4662,
  DEBUG 3F11, D 467A`). Handler 0x3F11 ustawia bit 0 `(IX-54h)` (0x8810),
  wykonuje resztę linii (ładuje program, jeśli podano) i skacze do **0x1564**
  (0x3F34 / 0x4032).
- Komunikat „H-ERROR - mikrosystem (M) ignoruj (I)” przy starcie (brak
  odpowiedzi kontrolera dyskietek) — wybór M → `JP 1564h` (0x0E2E). Czyli
  Bosman bez działającej stacji może pracować jako „mikrosystem” sterowany
  z drugiego komputera.

## Slave — pętla rozkazów (0x1564)  🟡

```
1564  DI ; SP=FBC0 ; SIOB_INIT z konfiguracją F36A
      zeruje F35F, FBF8 ; kopiuje stub rejestrów 1539.. → FBC0 (40 B), 3 B → F4CD
158D  wyślij 80h, 88h            ; „jestem gotów” (rozkaz 5Dh też tu skacze)
1597  pętla: odbierz bajt b; b-50h ∈ 0..10h → skok przez tablicę 15E3;
      rozkazy parzyste (50,52,…) odbierają dodatkowo 1 bajt parametru (w C)
```

| Rozkaz | Adres | Przypuszczalne działanie |
|---|---|---|
| 50 | 1605 | odbiór bloku do pamięci (adres/licznik wcześniej ustawione) |
| 51 | 1614 | porównanie bloków pamięci |
| 52 | 1638 | wysłanie bloku z sumą kontrolną |
| 53–55 | 164E, 1653, 1662 | ? |
| 56 / 58 | 1667 / 1687 | ustaw młodszy / starszy bajt adresu (parametr) |
| 57 | 166C | ? |
| 59, 5A | 1694, 1698 | ? (5A — z parametrem, używany przez mastera) |
| 5B | 16A1 | odczyt bajtu spod adresu (z przełączaniem banków, `F452`) i wysłanie |
| 5C | 16AF | ? |
| 5D | 158D | ponowne „hello” → 80h 88h |
| 5E | 03DA | restart systemu (powrót do CCP) |
| 5F | 16BD | ? |
| 60 | 1724 | ? (master wysyła 60h 00h po połączeniu) |
| — | 15BA | punkt wejścia po pułapce/breakpoincie: zapis wszystkich rejestrów do FBC0.., wysłanie 81h |
| — | 1554 | przywrócenie rejestrów z FBC0.. i skok (uruchomienie kodu) |

## Master — dostęp do D/E/F (0x0476)  🟡

- `F267` = „trzeba (ponownie) połączyć”; **wartość początkowa 1Ah** (z obrazu
  BIOS w ROM 0x2D67), więc pierwsze odwołanie do D/E/F zawsze uruchamia
  połączenie; po udanym połączeniu = 0, po timeoucie znów 1Ah.
- Połączenie: pytanie „Odwołanie do dysku w D E F - docelowy (D) rezygnuj (R)”,
  `F268 |= C0h` (zajęte), init SIO-B, wysyła **5Dh**, czeka na **88h**,
  potem czyta pamięć slave'a: DPB → `F2AB`, 60 bajtów → `F9A0`, maskę
  napędów slave'a (`F370`) przesuniętą o 3 bity → bity 3–5 lokalnego `F370`
  (D/E/F = napędy slave'a A/B/C), na koniec wysyła 60h 00h.
- Wybór D–F używa tablicy DPH od `F4A0` (zamiast `F27B`).
- Bajty idą przez `C_PUNCH` (0x1247 → SIO-B), odbiór `0x12E0`, timeout w `FA19`.
- Błąd: „Brak komunikacji z drugim komputerem”; polecenie „Zwolnienie dysków
  D E F” (narzędzia CCP) zwalnia łącze.

## Stan w emulatorze

Dwa procesy `bosman8` z `--aux tcp-listen:…` / `--aux tcp:…`; na slave `DEBUG`.
- ✅ Slave wchodzi w tryb rozkazów, master łączy się (pytanie D/R, brak
  komunikatu o braku komunikacji).
- ❌ `DIR E:` → „Brak pliku” (slave ma dyskietkę z 14 plikami w B:),
  `DIR D:` → „brak napędu”. Do zbadania: semantyka pozostałych rozkazów,
  czy master wykonuje BIOS slave'a, timing SIO-B / przełączanie banków
  podczas zdalnego odczytu, ewentualne braki w emulacji (np. przerwania, NMI
  slave'a przy operacjach dyskowych wykonywanych zdalnie).
