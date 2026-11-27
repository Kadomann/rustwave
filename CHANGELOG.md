# Änderungen

## 0.9.1 – 2026-11-27

### SSTV
- Dekoder deutlich robuster gegen Rauschen: Bilder kommen in der Simulation bis etwa −1 bis −2 dB Störabstand (2,4-kHz-Band) vollständig an, bisher war bei etwa +5 dB Schluss. Bei mittlerem Rauschen ist das Bild sichtbar ruhiger (PSNR +1,5 bis +2,5 dB).
- Phasendiskriminator mit 0,7 ms Verzögerung und Mittelung je Pixel statt geklemmter Momentanfrequenz: keine Klicks, kein Kontrastverlust im Rauschen
- VIS-Erkennung: Feinlage über das ganze Bitmuster, unsichere Bits werden verworfen, Verstimmung bis ±120 Hz wird gemessen und im Bild ausgeglichen
- Sync: beide Flanken ausgewertet, Zeilendauer mit Vorwissen geschätzt (wenige verrauschte Syncs kippen das Bild nicht mehr schräg), Wiedereinrasten auch bei schwachem Sync
- Neuer Messstand `cargo test --release sstv_rauschen -- --ignored --nocapture` und Test gegen Fehlstarts bei bandbegrenztem Rauschen

## 0.9.0 – 2026-11-21

Erste öffentliche Version. Gegenüber dem Ausgangsstand (früher „SSTV Station“, SSTV/RTTY/CW/APRS mit CAT und IC-705 über WLAN) neu:

### SSTV
- Weitere Modi: Martin M3/M4, Wraase SC2-120/180, Pasokon P3/P5/P7, Robot 8/24 S/W (insgesamt 23)

### POCSAG und DAPNET
- Neuer Reiter POCSAG: 512/1200/2400 Bd, Ausgabemodi, Zeichensätze und Schalter wie multimon-ng, Senden
- Empfang über den IC-705 verbessert: mehrere parallele Empfangswege, Entzerrer, Soft-Decoding, Diagnose
- ZF-Empfang: FM-Demodulation aus der 12-kHz-ZF des IC-705; automatische Umschaltung NF/ZF per CI-V (IC-705, IC-7300, IC-9700; WLAN bzw. USB)
- DAPNET-Ansicht: Rubriken (Liste von pi-star.de), Zusammenführen von Klartext- und Skyper-Weg, Skyper-Kopf entschlüsselt, Zeitsignale und Dienst-RICs ausblendbar, RIC-Tooltips, gehörte Sender mit Pegel
- Ruf ins Netz über die DAPNET-API; Betrieb als DAPNET-Sender (UniPager-Protokoll, Zeitschlitze)

### Packet Radio
- Neuer Reiter: AX.25 2.0 mit Verbindungen, 300/1200 Bd AFSK, 9600 Bd G3RUH
- TNC für andere Programme: KISS über TCP und AGW

### APRS
- Stationskarte per Mouse-over: Kompassrose, Position mit Locator, Kurs/Tempo, Höhe, PHG, Mic-E-Status, Gerät, Weg, Rohpaket
- Wetter aus allen Formaten, gegliedert mit Taupunkt, gefühlter Temperatur, Beaufort, Drucktendenz und Verlaufskurven
- Spuren, Funkwege, Telemetrie, Objekte, Hinweise/Alarme, automatische Antworten auf Abfragen, GPX/KML-Export
- SmartBeaconing-Profile, Höhe in der Bake, Mic-E-Bake; GPS-Position auch über WLAN

### Funkgerät und Betrieb
- S-Meter-Abfrage, Pegel je APRS-Station und DAPNET-Sender
- Sprechfunk über WLAN/USB (PC-Mikrofon und -Lautsprecher)
- Satelliten und ISS: Bahndaten (CelesTrak/AMSAT/SatNOGS), Überflüge, Doppler-Nachführung, ISS-Voreinstellungen mit änderbaren Frequenzen
- Layout der Funkgerät-Leiste bei schmalen Fenstern

### Projekt
- Lizenz GPL-3.0-or-later, Dokumentation für GitHub, Build über GitHub Actions
