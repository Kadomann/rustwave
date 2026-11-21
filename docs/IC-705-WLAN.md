# Icom IC-705 über WLAN

Rustwave steuert das IC-705 vollständig über WLAN: CAT (CI-V), Empfangs- und Sendeaudio, Sendetaste (PTT) und GPS laufen über das Netzwerk, ein USB-Kabel ist nicht nötig. Das gilt im Heimnetz ebenso wie direkt mit dem IC-705 als Access Point (unterwegs, ohne Router).

> **Getestet und verifiziert** mit einem echten IC-705: Empfang und Senden in allen Betriebsarten, CAT, PTT, ZF-Ausgang, GPS-Position über WLAN und APRS-Tracking mit SmartBeaconing. Andere Icom-Geräte mit LAN (IC-9700, IC-7610, IC-905) verwenden dasselbe Protokoll, sind aber ungetestet.

## Was über WLAN läuft

| Funktion | Weg |
|---|---|
| Frequenz, Mode, Leistung, Messwerte, S-Meter, Spektrum | CI-V über den Steuerkanal (UDP 50001/50002) |
| Empfangsaudio (NF oder 12-kHz-ZF) | Audiokanal (UDP 50003), 48 kHz, 16 Bit |
| Sendeaudio | Audiokanal, in 20-ms-Paketen |
| PTT | CI-V |
| GPS-Position und -Uhrzeit | CI-V – damit APRS-Baken mit Kurs, Tempo und Höhe sowie SmartBeaconing |

## Einrichtung im Heimnetz

**Am IC-705**

1. `MENU > SET > WLAN Set`: mit dem WLAN verbinden. Die IP-Adresse steht im WLAN-Menü bzw. in der Geräteliste des Routers. Eine feste Adresse (DHCP-Reservierung im Router) ist praktisch.
2. `MENU > SET > WLAN Set > Remote Settings`: **Network Control = ON**, unter *Network User1* Benutzername und Passwort anlegen. Die UDP-Ports bleiben auf 50001/50002/50003.
3. `MENU > SET > Connectors > MOD Input`:
   - **DATA MOD = WLAN** für die Datenbetriebsarten (Senden in USB-D/FM-D),
   - **DATA OFF MOD = WLAN**, wenn ohne DATA gesendet wird (Senden in USB/FM, auch für den Sprechfunk am PC).
4. Die CI-V-Adresse bleibt `A4h`.

**In Rustwave**

*Einstellungen › Transceiver*: **WLAN / LAN** wählen, Gerät *Icom IC-705*, IP-Adresse, Benutzer und Passwort eintragen, *Verbinden*. Die Funkgerät-Leiste zeigt danach „Audio WLAN“. Beim nächsten Start verbindet Rustwave von selbst wieder.

Windows fragt beim ersten Verbinden eventuell, ob Rustwave im Netzwerk kommunizieren darf – für private Netzwerke zulassen.

## Direkt ohne Router (IC-705 als Access Point)

Für unterwegs: Das IC-705 spannt selbst ein WLAN auf, der PC verbindet sich damit.

1. Am IC-705: `MENU > SET > WLAN Set`: WLAN OFF › *Connection Type* = **Access Point** › *Connection Settings (Access Point)*: SSID und Passwort festlegen › WLAN ON. Netzwerkbenutzer unter *Remote Settings* wie oben.
2. In Rustwave unter *Einstellungen › Transceiver › WLAN / LAN* auf **Direkt (Access Point)** umschalten. Das Funkgerät hat dann immer die Adresse `192.168.59.1`.
3. WLAN-Name und -Passwort des IC-705 eintragen und **PC mit IC-705-WLAN verbinden** drücken: Rustwave legt das Windows-WLAN-Profil an, verbindet den PC und danach das Funkgerät. Ein grüner Haken zeigt, dass der PC im Funkgeräte-WLAN ist.

Es kann nur ein Gerät verbunden sein, und der PC hat währenddessen über WLAN kein Internet. Bekommt der PC keine Adresse, dem WLAN-Adapter eine feste Adresse geben (z. B. `192.168.59.10`, Maske `255.255.255.0`).

## ZF-Ausgang für FM-Daten

Über den NF-Ausgang hört man beim IC-705 FM nur nach Deemphasis und einem Tiefenfilter (Low Cut). Für POCSAG ist das Signal danach praktisch unlesbar, auch APRS und Packet verlieren etwas. Das IC-705 kann stattdessen die **12-kHz-ZF** ausgeben; Rustwave demoduliert FM dann selbst.

- **Automatisch** (Voreinstellung, *Audio-Menü der Funkgerät-Leiste › ZF am Funkgerät automatisch umschalten*): Bei POCSAG, APRS UKW und Packet 1200/9600 stellt Rustwave den Ausgang per CI-V auf ZF, bei SSTV, RTTY, CW, APRS KW und Packet 300 auf NF. Umgeschaltet wird der Ausgang der aktuellen Verbindung – über WLAN Menü `01 14` (*WLAN AF/IF Output*), über USB `01 09`. Beim Trennen und Beenden stellt Rustwave wieder NF ein.
- **Ergebnis an einem echten DAPNET-Sender:** über NF fast nur Zeitansagen starker Sender, über ZF rund 11 Rufe pro Minute einschließlich Rubriken.
- Den **Pegel** der ZF stellt man am Gerät unter `SET > Connectors > WLAN AF/IF Output > IF Output Level` (bzw. `USB AF/IF Output` über Kabel) ein; die POCSAG-Diagnose in Rustwave meldet Übersteuerung.

## Sprechfunk am PC

🔊 in der Funkgerät-Leiste: Empfang über den PC-Lautsprecher. **Sprechen** gedrückt halten oder die Leertaste: Das PC-Mikrofon geht über WLAN zum IC-705. Dafür am Gerät *DATA OFF MOD = WLAN* und in FM/SSB ohne DATA senden. Mikrofon, Pegel und Lautsprecher unter *Einstellungen › Audio*.

## Andere Programme über Rustwave

*Einstellungen › TNC*: KISS über TCP (Port 8001) bzw. AGW (Port 8000). Winlink Express, YAAC, APRSIS32, Xastir & Co. senden und empfangen dann Packet bzw. APRS über das IC-705 – drahtlos über Rustwave.

## Fehlersuche

| Problem | Ursache / Abhilfe |
|---|---|
| Verbindung kommt nicht zustande | Nur **eine** Netzwerkverbindung gleichzeitig: wfview, RS-BA1 oder SR5-Programme vorher trennen. Network Control = ON? Benutzer und Passwort richtig? Firewall erlaubt Rustwave? |
| Ende der Aussendung fehlt | Das IC-705 puffert Sendeaudio etwa 300 ms. Rustwave löst die PTT deshalb erst danach. Fehlt trotzdem etwas, die WLAN-Verbindung prüfen (Selbsttest). |
| Aussetzer im Empfang | Die APRS-Anzeige zählt nachgeforderte und verlorene Audiopakete. Bei hohem Verlust: besserer WLAN-Empfang, Access-Point-Betrieb oder USB-Kabel. |
| Verbindung reißt ab | Rustwave verbindet nach 5, 10, 20 und dann alle 30 Sekunden neu (abbrechbar). |
| Keine GPS-Position | Am IC-705 GPS einschalten (`MENU > GPS > GPS Set > GPS Select`) und Satellitenempfang abwarten. |
| POCSAG bleibt stumm | ZF-Automatik eingeschaltet? Bei Übersteuerung den IF Output Level verringern. |
| *Hilfe › Selbsttest* | prüft Audio, Funkgerät, WLAN-Verluste, Access Point, GPS und Uhr; der Bericht hilft bei Fehlermeldungen. |

## Zum Protokoll

Icom hat das Netzwerkprotokoll nicht offengelegt. Die Umsetzung in `src/icomnet.rs` folgt **kappanhang** (MIT-Lizenz, HA2NON/ES1AKOS), die Paketaufbauten sind mit **wfview** abgeglichen. Für automatische Tests ohne Funkgerät simuliert `tools/fake_ic705.py` ein IC-705 (siehe [BAUEN.md](BAUEN.md)).
