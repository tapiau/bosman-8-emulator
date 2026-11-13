# Zadania

Legenda: ✅ zrobione · 🔄 w trakcie · ⏳ zaplanowane · ❓ wymaga danych/decyzji

## Faza 0 — rozpoznanie
- ✅ Przegląd dokumentacji `bosman-8-rom` i `bosman-8-ang-3000-rom`
- ✅ Ustalenie mapy pamięci i portów Bosmana z kodu ROM (zob. [bosman8-hardware.md](bosman8-hardware.md))
- ✅ Ustalenie wideo/klawiatury/UART terminala (zob. [ang3001-hardware.md](ang3001-hardware.md))
- ✅ Wybór technologii: Rust + iz80 (zob. [ARCHITECTURE.md](ARCHITECTURE.md))

## Faza 1 — szkielet
- ✅ T1.1 Workspace Cargo, crate'y `b8-link`, `b8-chips`, `bosman8`, `ang3001`
- ✅ T1.2 `b8-link`: trait endpointu + backendy null / TCP listen / TCP connect / stdio / serial hosta
- ✅ T1.3 `b8-chips`: 74LS259, 8253, Z80-SIO (tryb bajtowy), 8251

## Faza 2 — Bosman-8
- ⏳ T2.1 Pamięć: ROM 32 KB / 8 banków RAM 32 KB w dolnej połowie, wspólne górne 32 KB
- ⏳ T2.2 Porty: zatrzask 0x00–0x0F, SIO 0x80–0x83, 8253 0x84–0x87, DIP 0x98
- ⏳ T2.3 Binarka CLI: `--term tcp-listen:…|stdio|serial:…`, `--aux …` (SIO-B), `--trace-io`
- ⏳ T2.4 Boot do znaku zachęty CCP z RAM-dyskiem A:
- ⏳ T2.5 WD1770 + obrazy dyskietek (B:, C:) ❓ format fizyczny dyskietki do ustalenia
- ⏳ T2.6 Centronics (port 0x98 OUT) → plik

## Faza 3 — ANG-3001
- ⏳ T3.1 Pamięć: ROM 16 KB + RAM, wideo 512×256 z 0x8000, scroll port 0x80
- ⏳ T3.2 8251 na 0x90/0x91, timer 0x88–0x8B (dźwięk — na początek tylko „bip” w logu)
- ⏳ T3.3 Klawiatura: porty 0x8C/0x8D/0x8E, VSYNC na bicie 2 portu 0x8E
- ⏳ T3.4 Okno (minifb), mapowanie klawiszy hosta, BREAK, LOCAL
- ⏳ T3.5 Weryfikacja ekranu startowego (baner „ANG-3000”) i offsetu wierszy

## Faza 4 — integracja
- ⏳ T4.1 Bosman ↔ ANG po TCP: znak zachęty `A>` na ekranie terminala
- ⏳ T4.2 Uruchomienie demek z `bosman-8-ang-3000-rom/demo` (wstrzyknięcie plików na RAM-dysk)
- ⏳ T4.3 Narzędzie do wgrywania plików na RAM-dysk / obrazy dysków

## Faza 5 — później
- ⏳ Dźwięk (cpal) w terminalu
- ⏳ Wersja webowa (WASM + canvas, łącze przez WebSocket albo oba emulatory w jednej stronie)
- ⏳ Debugger (breakpointy, podgląd pamięci)

## Dziennik
(uzupełniany na bieżąco — najnowsze na dole)
- 2026-11-13: rozpoznanie sprzętu z kodu ROM; dokumentacja w `doc/`; szkielet workspace z interfejsami
  (`b8-link`, `b8-chips`). T1.2 i T1.3 zlecone równolegle subagentom; T2/T3 ruszą po nich.
- 2026-11-13: T1.2 (`b8-link`, 6 testów) i T1.3 (`b8-chips`, 58 testów) gotowe. Druga fala: T2.x (Bosman) i T3.x (ANG) równolegle u subagentów.
