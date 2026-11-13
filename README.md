# Bosman-8 + ANG-3001 — emulator

Emulatory polskiego mikrokomputera **Bosman-8** (DZM-180, Z80A, CPM-R) i jego
terminala graficznego **ANG-3001** (Z80B, 512×256). Dwa osobne programy
połączone wirtualnym łączem szeregowym (domyślnie TCP); każdy kanał szeregowy
można też przepiąć na port szeregowy hosta, stdio albo zostawić pusty.

## Szybki start

```sh
cargo build --release
./target/release/bosman8          # nasłuchuje terminala na tcp 127.0.0.1:3001
./target/release/ang3001          # okno terminala, łączy się z 127.0.0.1:3001
```

Na ekranie terminala pojawi się baner i znak zachęty `A>`.

Przykłady:

```sh
bosman8 --term stdio                          # Bosman bez terminala graficznego (surowe ESC w konsoli)
bosman8 --disk-b b.img --printer lpt.txt      # dyskietka B: (tworzona, jeśli nie istnieje) i drukarka do pliku
bosman8 --aux serial:/dev/ttyUSB0@9600        # SIO-B (V.24) na prawdziwy port szeregowy
ang3001 --link serial:COM3@115200             # terminal na port szeregowy hosta
ang3001 --load-ter ../bosman-8-ang-3000-rom/demo/cube/CUBE.TER   # demo wgrane wprost do terminala
```

Specyfikacje łącza: `none`, `loopback`, `stdio`, `tcp-listen:ADDR:PORT`,
`tcp:HOST:PORT`, `serial:DEV[@BAUD]`. Pełna lista opcji: `--help`.

Klawisze terminala: F12 (trzymany) = BREAK, F11 = reset, F10 = CAPS, F9 = turbo.

## Dokumentacja

- [doc/ARCHITECTURE.md](doc/ARCHITECTURE.md) — architektura, model łącza
- [doc/TASKS.md](doc/TASKS.md) — plan zadań i dziennik postępu
- [doc/bosman8-hardware.md](doc/bosman8-hardware.md) — sprzęt Bosmana ustalony z ROM-u
- [doc/ang3001-hardware.md](doc/ang3001-hardware.md) — sprzęt terminala

ROM-y (`roms/`) pochodzą z repozytoriów analizy `bosman-8-rom` i `bosman-8-ang-3000-rom`.
