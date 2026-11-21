# Selbst bauen und testen

Rustwave ist ein reines Rust-Programm (eframe/egui für die Oberfläche, cpal für Audio). Gebaut wird mit `cargo`; getestet ist der Bau mit Rust 1.97.

## Windows

1. [Rust](https://rustup.rs) installieren (dabei die „Visual Studio Build Tools“ mit C++ zulassen).
2. Im Projektordner:
   ```
   cargo build --release
   ```
   Ergebnis: `target\release\rustwave.exe`, das Programmsymbol wird über `build.rs` eingebettet.

## Linux (Debian/Ubuntu)

```
sudo apt install build-essential pkg-config libasound2-dev
cargo build --release
```

Unter Linux laufen alle Betriebsarten, Audio über ALSA/PulseAudio/PipeWire und CAT über serielle Schnittstellen bzw. WLAN. Windows-eigene Teile (Windows-Standort, WLAN-Profil für den Access-Point-Betrieb, Bildschirmtastatur, Dateidialoge) fehlen dort; WAV-Dateien zieht man stattdessen ins Fenster.

## Windows-Programm unter Linux bauen

```
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

Der Linker ist in `.cargo/config.toml` eingetragen. Ist die Windows-Standardbibliothek über rustup nicht erreichbar, lässt sie sich aus den Quellen bauen:
```
RUSTC_BOOTSTRAP=1 cargo build --release --target x86_64-pc-windows-gnu -Zbuild-std=std,panic_abort
```

## Tests

```
cargo test --release
```

prüft Kern, Modems und Protokolle: SSTV-Modi (Kodieren und Dekodieren), RTTY, CW, APRS (auch mit Rauschen und Taktfehlern), Mic-E, Telemetrie, Wetter, POCSAG (BCH, Rauschen, Lautsprecher-Audio, ZF-Weg, multimon-ng-Formate), AX.25 mit Verbindung über das Modem, KISS/AGW über TCP, DAPNET-Protokoll gegen einen nachgebauten Kern, CAT-Abfragen, Satellitenbahnen (gegen skyfield), Logbuch und Vorlagen.

Dazu kommen Vergleichstests mit anderen Programmen und echten Aufnahmen, die mit `--ignored` gestartet werden:

```
# Netzwerk Ende-zu-Ende gegen ein simuliertes IC-705 (rx.wav = Robot 36, 48 kHz mono)
python3 tools/fake_ic705.py --rx-wav /tmp/f/rx.wav --tx-wav /tmp/f/tx.wav --report /tmp/f/report.json &
SSTV_FAKE_DIR=/tmp/f cargo test --release gegen_simuliertes -- --ignored --nocapture

# SSTV-Testbild als WAV (z. B. für andere Dekoder)
SSTV_WAV=/tmp/r36.wav SSTV_MODE="Robot 36" cargo test --release export_wav -- --ignored

# POCSAG gegen multimon-ng
POCSAG_RAW=/tmp/p.raw cargo test --release export_raw -- --ignored
multimon-ng -t raw -a POCSAG512 -a POCSAG1200 -a POCSAG2400 /tmp/p.raw

# POCSAG aus einer ZF-Aufnahme des IC-705
ZF_WAV=aufnahme.wav cargo test --release zf_wav_dekodieren -- --ignored --nocapture

# Packet Radio gegen Dire Wolf
PACKET_WAV=/tmp/pk.wav PACKET_SPEED=9600 cargo test --release export_packet -- --ignored
atest -B 9600 /tmp/pk.wav

# KISS gegen Dire Wolfs kissutil (zweites Terminal: kissutil -h 127.0.0.1 -p 18002)
cargo test --release kiss_interop -- --ignored --nocapture

# APRS-Testaufnahme mit typischen Paketen (Mobil, Mic-E, Wetter, Telemetrie, Objekt)
APRS_DEMO=/tmp/aprs.wav cargo test --release aprs_demo_wav -- --ignored
```

## Aufbau des Quelltexts

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Anwendung, Zustand, Senden, Ereignisse |
| `src/sstv.rs`, `src/template.rs`, `src/editor.rs` | SSTV-Modi, Kodierer/Dekoder, Vorlagen und Editor |
| `src/rtty.rs`, `src/rtty_ui.rs`, `src/cw*.rs` | RTTY und CW |
| `src/aprs.rs`, `src/aprsinfo.rs`, `src/aprs_telem.rs`, `src/aprs_export.rs`, `src/aprs_ui.rs` | APRS-Modem, Pakete im Klartext, Telemetrie, Export, Oberfläche |
| `src/pocsag.rs`, `src/pocsag_ui.rs`, `src/dapnet.rs`, `src/dapnet_tx.rs` | POCSAG-Dekoder/-Kodierer, DAPNET-Rubriken, DAPNET-Sender und -API |
| `src/ax25.rs`, `src/packet_ui.rs`, `src/kiss.rs` | AX.25, Packet-Terminal, KISS/AGW-Server |
| `src/icomnet.rs` | Icom-Netzwerkprotokoll (WLAN/LAN) |
| `src/ifdemod.rs`, `src/ifswitch.rs` | FM aus der 12-kHz-ZF, Umschalten des Gerätausgangs |
| `src/cat.rs`, `src/rigpoll.rs`, `src/radio.rs`, `src/rigbar.rs`, `src/gps.rs` | CAT-Protokolle, Abfragen, Funkgerät-Leiste, GPS |
| `src/sat.rs` | Satellitenbahnen, Überflüge, Doppler |
| `src/voice.rs`, `src/audio.rs`, `src/rx.rs` | Sprechfunk, Audio, Empfangsthread |
| `src/logbook.rs` | Logbuch und ADIF |
| `tools/` | simuliertes IC-705, CW-Testsignale, Liste der Drittlizenzen |

## Release bauen

Ein Git-Tag `v…` löst in GitHub Actions den Bau aus (`.github/workflows/build.yml`): Tests unter Linux, Windows-Programm, ZIP mit `rustwave.exe`, Lizenzen und Kurzanleitung als Release-Anhang.

Die Liste der Drittlizenzen nach Änderungen an den Abhängigkeiten neu erzeugen:
```
python3 tools/third_party.py > docs/THIRD-PARTY.md
```
