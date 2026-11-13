# Obrazy dyskietek

Obrazy w formacie emulatora: surowe sektory (cylinder, strona, sektor),
system plików CP/M 2.2 zgodny z tablicami DPB w ROM-ie Bosmana.
Tworzone i edytowane narzędziem `b8disk` (`cargo run --release --bin b8disk -- help`).

## narzedzia.img — 200 KB (domyślny format B:/C:, DIP 0x40)

Narzędzia z `bosman-8/software/soft/cpm-utilities`:

| Plik | Do czego |
|---|---|
| PIP.COM | kopiowanie plików (`PIP A:=B:*.*`) |
| STAT.COM | wolne miejsce, rozmiary, atrybuty |
| ED.COM | edytor tekstu |
| SUBMIT.COM, XSUB.COM | pliki wsadowe |
| ZSID.COM | debugger Z80 |
| KATALOG.COM | katalog (po polsku) |
| WYDRUK.COM | drukowanie plików (po polsku) |
| DUU.COM | edytor sektorów / narzędzie dyskowe |
| FILCOM.COM | porównywanie plików |
| MF.COM, MFU.COM | kopiowanie/przenoszenie plików |
| UNERASE.COM | odzyskiwanie skasowanych plików |
| TRPLIK.COM | transfer plików |

Pominięte: SID (zastępuje go ZSID), FORMAT (operuje na sprzęcie, nieprzetestowany
w emulatorze), TESTRAM. Na dyskietce zostaje 120 KB wolnego miejsca.

Użycie (emulator zapisuje zmiany do obrazu — pracuj na kopii, jeśli chcesz
zachować oryginał):

```sh
cp disks/narzedzia.img /tmp/b.img
bosman8 --disk-b /tmp/b.img
```

## Demo terminala ANG-3001

Dodatkowo na obrazie są pliki z `bosman-8-ang-3000-rom/demo`:

| Plik | Do czego |
|---|---|
| RUNTERM.COM | ładowanie i uruchamianie programów `.TER` na terminalu |
| CUBE.TER | demo obracającej się kostki 3D |

Przy obrazie zamontowanym jako B: i podłączonym terminalu ANG-3001:

```text
B:
RUNTERM CUBE
```

Reset terminala kończy demo.
