![Rustwave](../assets/logo/logo-dark.png)

# Rustwave – Handbuch

Ausführliche Beschreibung aller Funktionen. Überblick, Installation und Lizenz stehen in der [README](../README.md), die Einrichtung des IC-705 über WLAN in [IC-705-WLAN.md](IC-705-WLAN.md).

SSTV, RTTY, CW, APRS, POCSAG und Packet Radio mit CAT-Steuerung für die üblichen Transceiver, geschrieben in Rust (früher „SSTV Station“).

SSTV, RTTY, CW, APRS, POCSAG (Funkruf) und Packet Radio in einem Programm.

**Aufbau des Fensters:** oben die Menüleiste (Datei, Bearbeiten, Ansicht, Funkgerät, Einstellungen, Hilfe) mit den Reitern *Betrieb*, *Vorlagen* und *Logbuch*. Darunter die **Funkgerät-Leiste** mit allem, was man im Betrieb anfasst: Betriebsart, Frequenz (antippen: Eingabe, Mode, Bänder), Mode, Sendeleistung, Verbindung, Audio, Aufnahme und RX/TX. Darunter in jeder Betriebsart links der Empfang, rechts Senden bzw. QSO. Selten Gebrauchtes wie *Leeren* oder *WAV dekodieren* steckt im **…**-Knopf des jeweiligen Bereichs. Im Touch-Betrieb ersetzt ein **☰**-Knopf die Menüleiste.

Eigenständiger SSTV-Client für Windows: eine einzige `.exe`, ohne Browser, ohne Installation und ohne zusätzliche DLLs.

## Start
`rustwave.exe` per Doppelklick starten. Windows SmartScreen meldet beim ersten Start eventuell eine unbekannte App, weil die Datei nicht signiert ist. Dann auf „Weitere Informationen“ und „Trotzdem ausführen“ klicken.

Einmalig unter **Einstellungen** (Menü oder Strg+,) einstellen. Beim ersten Start öffnet sich das Fenster von selbst:
- **Station:** Rufzeichen, Name, QTH und Locator. Sie erscheinen über `%m`, `%o`, `%a` und `%l` in den Vorlagen.
- **Audio:** Eingang „Mikrofon (USB Audio CODEC)“ und Ausgang „Lautsprecher (USB Audio CODEC)“ wählen, dann auf *Audio starten* klicken.
- **Transceiver:** Gerät, COM-Port, Baudrate und PTT-Methode wählen und auf *Verbinden* klicken. Später reicht ein Klick auf die Verbindungsanzeige in der Funkgerät-Leiste.
- **APRS:** SSID, Symbol, Kommentar und Pfad der Bake.
- **Pegel:** *Abstimmton 1750 Hz* senden und den Sendepegel so einstellen, dass die ALC gerade nicht ausschlägt. Der Sendepegel wird für SSTV, RTTY, CW, APRS UKW, APRS KW, POCSAG und Packet Radio getrennt gespeichert.

## Ein QSO in drei Reitern

### Betrieb
1. **CQ** ist vorgewählt. *Senden* schickt die CQ-Vorlage.
2. Sobald jemand antwortet, sein **Rufzeichen** oben eintippen. Kennt das Logbuch die Station, werden Name und QTH vorausgefüllt, und die Leiste springt auf **Antwort**.
3. *Senden*. Nach jedem fertig gesendeten Bild springt die Leiste zur nächsten Phase: Antwort → **Rapport** → **73**.
4. Beim Senden der **73**-Vorlage wird das QSO automatisch geloggt. *Neues QSO* leert die Felder und geht zurück zu CQ.

Die Vorschau zeigt immer genau das Bild, das gesendet wird, mit den aktuellen QSO-Daten. Unter der Vorschau liegen alle Vorlagen als **freie Ablage**. Ein Klick wählt eine Vorlage, auch außerhalb der Reihenfolge.
Ein empfangenes Bild wird automatisch zum **Bild im Bild** für die Rapport-Vorlage. Ein Klick auf ein anderes Galeriebild wählt dieses, ein Rechtsklick öffnet die Datei. Ein ins Fenster gezogenes Foto wird zum Sendefoto.
**FSKID:** Wie bei MMSSTV wird nach jedem Bild das eigene Rufzeichen als kurze FSK-Kennung angehängt (abschaltbar mit *FSKID senden*). Empfangene FSKIDs (auch von MMSSTV) landen automatisch in der QSO-Leiste, wenn dort noch kein Rufzeichen steht.

### RTTY
Oben auf **RTTY** umschalten. Die QSO-Leiste, das Logbuch, CAT/WLAN und die Phasen CQ → Antwort → Rapport → 73 bleiben dieselben.
- **Abstimmen:** In den Wasserfall auf die Mitte zwischen den beiden Spuren klicken (oder dort ziehen). Die grüne Linie markiert Mark, die orange Space.
- **Standard:** 45,45 Baud, 170 Hz Shift, Baudot/ITA2 (US-Ziffern wie in fldigi), Mark 2125 / Space 2295 Hz. Nach Konvention arbeitet das Funkgerät dabei in **LSB**: Die Schnellwahl stellt LSB ein. *Umkehren* tauscht Mark und Space für USB-Betrieb.
- **Weitere Einstellungen:** 50 und 75 Baud, Shift 200/425/850 Hz.
- **Rauschsperre:** Text erscheint erst ab dieser Signalgüte (grüner Balken). Niedriger eingestellt werden schwächere Signale lesbar, bei reinem Rauschen erscheinen dann aber Zufallszeichen.
- **Empfangsfenster:** Unterstrichene Rufzeichen übernimmt ein Klick in die QSO-Leiste, inklusive Vorausfüllen aus dem Logbuch und Sprung zur Antwort. Eigene Aussendungen erscheinen orange.
- **Senden:** Zu jeder Phase gibt es ein editierbares Makro mit Live-Vorschau. Es nutzt dieselben Makros wie die Vorlagen (`%c`, `%n`, `%r`, `%m` …). Dazu kommt ein Feld für freien Text. Nach dem Senden springt die Phase weiter, beim 73 wird das QSO als **RTTY** geloggt.
- **Geprüft:** Kodierer und Dekoder wurden mit dem unabhängigen Programm minimodem in beide Richtungen gegengeprüft.

### APRS
Oben auf **APRS** umschalten. Links oben wählt man das Band:
- **UKW 1200 Bd:** FM, Bell 202. Die Schnellwahl stellt Frequenz und FM ein: 144,800 MHz (Europa), 144,390 MHz (USA), 145,825 MHz (ISS). Den Sendepegel eher niedrig wählen, denn bei FM bestimmt er den Hub.
- **KW 300 Bd:** Töne 1600/1800 Hz (wie Direwolf). Schnellwahl nach george-smart.co.uk/aprs/aprs-on-hf: 40 m 7,0452 USB · 7,0486 / 7,0472 / 7,1040 LSB, 30 m 10,1476 USB, 20 m 14,1023 USB · 14,1030 / 14,1050 LSB, 10 m 28,1400 LSB. USB und LSB funktionieren beide, weil nur die Tonwechsel ausgewertet werden. Unter *1200 Bd* stehen zusätzlich 29,250 MHz FM und 28,1371 MHz USB (10 m). Sieben gegeneinander versetzte Dekoder lesen auch Stationen, die bis etwa ±100 Hz verstimmt sind. Pfad leer lassen oder `GATE`; WIDE-Digipeater gibt es auf Kurzwelle nicht. Sendepegel wie bei anderen SSB-Digimodes so einstellen, dass die ALC gerade nicht ausschlägt.
- **Gehörte Stationen:** Rufzeichen, Uhrzeit, Anzahl der Pakete, Entfernung und Richtung zur eigenen Position, Symbol und Kommentar. Gelesen werden unkomprimierte, komprimierte und Mic-E-Positionen, Objekte, Status und Nachrichten. Ein Klick auf ein Rufzeichen trägt es als Empfänger einer Nachricht ein.
- **Monitor:** alle Pakete im Klartext; eigene Aussendungen erscheinen orange. *WAV dekodieren* (oder eine WAV-Datei ins Fenster ziehen) liest z. B. eine Aufnahme.
- **Karte:** *Liste | Karte* schaltet auf eine Karte ohne Internet um: eigene Position in der Mitte, Entfernungsringe, Norden oben, Digipeater und Gateways orange. Antippen zeigt die Angaben der Station und setzt sie als Empfänger einer Nachricht; *+/−* ändert den Maßstab, *Auto* zeigt alle.
- **Positionsbake:** Rufzeichen kommt aus „Station“, dazu SSID (z. B. 9 für mobil), Position in Dezimalgrad (leer = Mitte des Locators; *vom Funkgerät* liest das eingebaute GPS von IC-705, IC-9700 oder IC-905, *Windows-Standort* nutzt den Windows-Ortungsdienst; optional wird das GPS vor jeder Bake neu gelesen), Symbol, Kommentar und Pfad (Standard `WIDE1-1,WIDE2-1`). *Bake senden* sendet sofort, *automatisch* alle 10, 20, 30 oder 60 Minuten oder **SmartBeaconing**: Mit dem GPS des IC-705 richtet sich der Abstand nach dem Tempo (im Stand alle 30 min, ab 90 km/h alle 3 min) und beim Abbiegen kommt eine zusätzliche Bake; in Fahrt werden Kurs und Tempo mitgesendet. Hört das Programm das eigene Paket über einen Digipeater wieder, wird das grün angezeigt.
- **Nachrichten:** An dich gerichtete Nachrichten erscheinen grün und werden nach 2,5 Sekunden automatisch bestätigt. Eigene Nachrichten werden bis zu dreimal im Abstand von 30 Sekunden gesendet, bis die Bestätigung eintrifft (✔ bestätigt / ✗ keine Bestätigung).
- APRS-Pakete sind keine QSOs und landen nicht im Logbuch.
- **Geprüft:** Von den 100 Paketen der Direwolf-Testdateien liest der Dekoder auf UKW 74 (Direwolf: 70), auf Kurzwelle 71 (Direwolf: 73). Alle gesendeten Pakete werden von Direwolf fehlerfrei gelesen.

### CW (Morse)
Oben auf **CW** umschalten. Das Programm dekodiert Morse und kann über ein Icom-Gerät (USB oder WLAN) auch senden; das QSO wird als **CW** geloggt.
- **Senden:** Wie bei RTTY gibt es zu jeder Phase (CQ, Antwort, Rapport, 73) ein Makro mit Vorschau und Dauer, dazu freien Text. Das Funkgerät tastet selbst (CI-V-Befehl 17); das Programm stellt dazu das Tempo (6–48 WpM) und Break-in auf *Semi*. Nach dem Senden springt die Phase weiter, beim 73 wird geloggt. *Stopp* bricht sofort ab. `^SK` sendet das Schlusszeichen ohne Zeichenpause.
- **Bandvorlagen** (Frequenz in der Funkgerät-Leiste antippen): Für jedes Band von 160 m bis 2 m stellt ein Knopf die Anfangsfrequenz des CW-Bereichs (IARU Region 1) und die Betriebsart CW ein, z. B. 40 m 7,000 MHz, 60 m 5,3515 MHz, 2 m 144,000 MHz.
- **Abstimmen:** Die Tonhöhe muss zum CW-Pitch des Funkgeräts passen (Icom: 600 Hz). Andere Stationen im Wasserfall anklicken oder *Stärkstes Signal* drücken.
- **Tempo** wird automatisch erkannt (etwa 5–60 WpM) und angezeigt; nach einem Wechsel zu einer Station mit ganz anderem Tempo kann das erste Wort fehlen.
- **Rauschsperre** in dB: Text erscheint erst, wenn das Signal so weit über dem Rauschen liegt.
- **Geprüft** mit erzeugten Signalen von 12 bis 40 WpM, Handtastung, Schwund, Tonablage, Nachbarsignal und Rauschen. Bei 6 dB Signal/Rauschen (in 500 Hz) liest der Dekoder 15–35 WpM praktisch fehlerfrei; das Vergleichsprogramm multimon-ng liest dort nichts mehr.

### POCSAG (Funkruf)
Oben auf **POCSAG** umschalten (oder F9). Empfangen werden Funkrufe wie im Amateurfunk-Netz DAPNET; der Dekoder arbeitet wie multimon-ng und hat dieselben Einstellungen.
- **Frequenzen** (Frequenz in der Funkgerät-Leiste antippen, stellt FM ein), nach der DAPNET-Frequenzliste: **439,9875 MHz** (Standard in DL, HB9, PA, ON, F, G, OK, SP, HA, LA, CT, S5, LZ, Russland, USA, Kanada), 438,025 und 439,930 MHz (Österreich), 433,625 MHz (Irland), 144,8625 MHz (Luxemburg, Spanien, Grenzregion DL/LX), 144,9375 und 144,5125 MHz (Italien), 144,975 MHz (Russland), 144,280 MHz (China), 144,390 und 144,4125 MHz (Thailand). DAPNET sendet mit 1200 Baud.
- **Dekoder wie multimon-ng:** POCSAG512, POCSAG1200 und POCSAG2400 laufen gleichzeitig (einzeln abwählbar). *Ausgabe* Standard (nach Funktionsbits: 0 numerisch, 1–3 Text), Automatisch, Numerisch, Alphanumerisch oder Skyper (`-f`), *Zeichensatz* US, DE, FR, DK, SE, SI (`-C`, z. B. DE: Ä Ö Ü ä ö ü ß), *Fehlerkorrektur* 0–2 Bit je Codewort (`-b`), *Polarität* automatisch, normal oder invertiert (`-P`), *leere ausblenden* (`-e`), *Unwahrscheinliches verwerfen* (`-u`), *Teilempfang zeigen* (`-p`). Darunter steht die passende multimon-ng-Befehlszeile. *Skyper-RICs* (voreingestellt 4520): Textrufe an diese RICs werden auch im Standard-Format gleich als Skyper entschlüsselt, der zweistellige Kopf erscheint getrennt in eckigen Klammern (als Hex). DAPNET verschiebt nur die Skyper-Rubriken auf RIC 4520 (0x11A8) um ein Zeichen; dieselben Inhalte gehen im Klartext an andere RICs.
- **Liste | Monitor:** Die Liste zeigt Zeit, Baudrate, RIC, Funktion (A–D) und Text, der Monitor dieselben Zeilen wie multimon-ng. Filter nach RIC oder Text. *Eigene RICs* erscheinen grün und werden in der Statuszeile gemeldet. *Mitschrift in Datei* schreibt jede Zeile mit Datum nach `Bilder\Rustwave\pocsag-JJJJMMTT.txt`. *WAV dekodieren* liest Aufnahmen.
- **Audio:** Am besten vom Daten- bzw. 9600-Bd-Anschluss (ohne Deemphasis). Audio vom Lautsprecher geht auch: Je Baudrate laufen dafür Varianten mit Deemphasis- und Hochpass-Ausgleich mit, und die Varianten stimmen über das Ergebnis ab. Die Rauschsperre darf offen sein, Rauschen erzeugt keine Fehlmeldungen.
- **Diagnose** (aufklappbar über der Liste): zählt je Baudrate erkannte Präambeln (ein POCSAG-Sender ist zu hören), Sync-Wörter und gute, korrigierte und defekte Codewörter. Daraus kommt ein Hinweis: *kein Signal* (Frequenz, Rauschsperre, Sender in Reichweite?) oder *Signal da, aber zu verzerrt* (Audioweg). Nach *WAV dekodieren* stehen dieselben Zahlen in der Statuszeile.
- **DAPNET-Ansicht:** Neben *Liste* und *Monitor* zeigt *DAPNET* alle Rufe, wobei Rubriken (Wetter- und DWD-Warnungen, DX-Cluster, Relaislisten, Notfunk …) mit Nummer und Bezeichnung erscheinen. DAPNET sendet jede Rubrik-Nachricht doppelt – im Klartext an RIC 1000 + Rubriknummer (AlphaPoc, Swissphone) und für Skyper an RIC 4520 mit Kopf aus Rubrik und Platz; Rustwave legt beide zu einer Zeile zusammen. Die Bezeichnungen stammen aus der Rubrikliste von pi-star.de; weil die Nummern je Land verschieden belegt sind (11 = Berlin in DL, Wien in OE), wählt man unter *Region* die eigenen Sendergruppen (Voreinstellung `dl-all`). *Zeigen* filtert auf alles, nur Rubriken oder eine Rubrik. Der Tooltip über der RIC (in Liste und DAPNET-Ansicht) erklärt bekannte RICs.
- **Zeitsignale und Dienst-RICs ausblenden** (beides voreingestellt, gilt für alle Ansichten): Zeitsignale sind RIC 200 (UTC, Swissphone), 208 (Ortszeit, Swissphone), 216 (UTC, AlphaPoc), 224 (Ortszeit, AlphaPoc) und 2504 (numerisch); Dienst-RICs sind 8 (Senderkennung) und 4512 (Skyper-Rubriknamen). Die Mitschrift-Datei bleibt vollständig.
- **Gehörte DAPNET-Sender:** In der DAPNET-Ansicht listet Rustwave die Sender, deren Kennung (RIC 8) ankam: Anzahl der Kennungen, zuletzt gehört, Rufe im selben Durchgang (geschätzt: bis ein Zeitschlitz nach der Kennung) und der Anteil fehlerfrei empfangener Rufe als Maß für die Empfangsqualität.
- **Ruf über DAPNET (Internet):** gibt einen Textruf (bis 80 Zeichen) über die REST-Schnittstelle von hampager.de (`http://www.hampager.de:8080/calls`) an DAPNET-Rufzeichen; ausgestrahlt wird er von den Sendern der gewählten Sendergruppen (z. B. `dl-all`). Braucht ein Benutzerkonto bei hampager.de.
- **Als DAPNET-Sender betreiben:** Rustwave meldet sich wie UniPager oder Pi-Star beim DAPNET-Kern an (TCP-Protokoll auf Port 43434, Voreinstellung `dapnet.afu.rwth-aachen.de`, dazu Ausweichserver im HAMNET), quittiert die Rufe und strahlt sie in den zugeteilten Zeitschlitzen (16 × 6,4 s) über das Funkgerät aus – mehrere Rufe am Stück, solange der Schlitz reicht, Vor- und Nachlauf (WLAN-Puffer) eingerechnet. Nötig sind ein eigener Sender-Zugang (Ticket „New Transmitter“ bei hampager.de: Rufzeichen des Senders und AuthKey), die Frequenz 439,9875 MHz in FM ohne Preemphasis und eine auf die Sekunde richtige PC-Uhr (eine bekannte GPS-Abweichung wird herausgerechnet). Mit *Nur RICs* sendet man – wie mit der POCSAG-Whitelist bei Pi-Star – nur Rufe an bestimmte RICs aus. Gesendet wird, solange der POCSAG-Reiter offen ist; Rufe, die länger als zehn Minuten warten, verfallen. Ein Sender ohne Aufsicht ist in Deutschland eine automatisch arbeitende Station und braucht eine eigene Zuteilung. Zugangsdaten liegen unverschlüsselt in den Einstellungen.
- **Am besten: ZF-Empfang** (Icom IC-705, IC-7300, IC-9700 u. a.): am Funkgerät *SET > Connectors > WLAN AF/IF Output* (bzw. *USB AF/IF Output*) *> Output Select* = **IF**, in Rustwave im Audio-Menü der Funkgerät-Leiste (oder im POCSAG-/Packet-Reiter) **ZF-Empfang** einschalten. Das Gerät liefert dann die 12-kHz-ZF; Rustwave demoduliert FM selbst, ohne Deemphasis und ohne den Hochpass gegen CTCSS – wie ein SDR. In einer echten DAPNET-Aufnahme (IC-705 über WLAN) kamen so 11 Rufe in einer Minute an, darunter Wetterwarnungen, Rubriken und Dienstmeldungen; über den NF-Weg waren es aus derselben Gegend nur die Zeitansagen der stärksten Sender. Rustwave erkennt selbst, ob ZF oder NF ankommt, und weist auf einen falsch gesetzten Schalter hin. Die Aufnahme (⏺) speichert die rohe ZF. Nur für FM: für SSTV, RTTY und CW in SSB braucht es NF.
- **Automatisch umschalten (Voreinstellung):** Ist ein IC-705, IC-7300 oder IC-9700 per CAT verbunden, stellt Rustwave *Output Select* selbst um – ZF bei POCSAG, APRS UKW und Packet 1200/9600, NF bei SSTV, RTTY, CW, APRS KW und Packet 300. Umgeschaltet wird der Ausgang der aktuellen Verbindung (IC-705: WLAN-Menü 01 14 bzw. USB 01 09; IC-9700: LAN 01 10 bzw. USB 01 05; IC-7300: ACC/USB 00 59, CI-V `1A 05`). Beim Trennen und Beenden stellt Rustwave wieder NF ein. Abschalten im Audio-Menü der Funkgerät-Leiste („ZF am Funkgerät automatisch umschalten“); lehnt das Gerät den Befehl ab, schaltet sich die Automatik selbst ab. WAV-Dateien mit ZF erkennt Rustwave beim Dekodieren am Spektrum.
- **Empfang über den NF-Weg (z. B. IC-705 über USB oder WLAN):** POCSAG ist ein Basisbandsignal mit viel Anteil unter 300 Hz; der normale NF-Weg der Funkgeräte (Deemphasis, Hochpass gegen CTCSS) schneidet genau das weg. multimon-ng liest so ein Signal nicht mehr. Rustwave hat dafür einen **Entzerrer je Aussendung**: Er erkennt die Präambel, lernt an Präambel und Sync-Wort, wie das Funkgerät das Signal verbogen hat, lernt mit jedem bestätigten Codewort weiter und führt den Bittakt nach (auch bei Sendern mit 0,4 % Taktfehler). Unsichere Bits werden mit den weichen Werten nachkorrigiert (Chase-Dekodierung); Adresswörter nur, wenn das Ergebnis eindeutig ist, damit nie eine falsche RIC erscheint. Der Gelernte gilt weiter, denn die Verzerrung kommt vom eigenen Gerät. In einer echten DAPNET-Aufnahme (IC-705 über WLAN, 439,9875 MHz) liest Rustwave so die starken Sender (Zeitansagen an RIC 200, 216, 2000, 2504), multimon-ng nichts. Schwache Sender gehen im NF-Weg im Rauschen unter: Am besten bleibt Audio vom Daten- bzw. 9600-Bd-Anschluss bzw. Datenmodus (FM-D/DATA) mit breitem Filter, CTCSS/Tonsquelch aus, Audiopegel so, dass nichts übersteuert.
- **Funkruf senden:** RIC, Funktion (0–3 bzw. A–D), Art (Text, numerisch, nur Ton) und Baudrate wählen, Text eingeben, *Funkruf senden*. Ein Klick auf eine RIC in der Liste übernimmt sie. „Rufzeichen anhängen“ ergänzt `de <Rufzeichen>`. Gesendet wird das POCSAG-Basisband (NRZ mit 576 Bit Präambel) für den Modulationseingang in FM; sauber geht das über den Daten-/9600-Anschluss bzw. FM-D. Den Sendepegel so wählen, dass der Hub etwa ±4,5 kHz beträgt. Auf der DAPNET-Frequenz senden die Sender des Netzes in Zeitschlitzen: zum Testen eines eigenen Pagers besser eine freie Frequenz nehmen.
- Funkrufe sind keine QSOs und landen nicht im Logbuch.
- **Geprüft:** Alles, was Rustwave sendet (512/1200/2400 Bd, Text, Ziffern, Nur-Ton, über mehrere Batches), liest multimon-ng 1.3 fehlerfrei. Mit Rauschen (auf 3 kHz begrenzt) liest Rustwave von 12 Rufen bei σ 0,4 alle (multimon-ng 10), bei σ 0,6 sieben (multimon-ng keinen), ohne falsche Meldungen.

### Packet Radio
Oben auf **PACKET** umschalten (oder F10). Rustwave arbeitet dann wie ein TNC mit Terminal: Monitor aller Rahmen, eine Verbindung zu einer Mailbox, einem Knoten, einem DX-Cluster oder einer anderen Station, und Text ohne Verbindung (Unproto).
- **Geschwindigkeit:** *1200 Bd FM* (AFSK 1200/2200 Hz, Standard auf 2 m und 70 cm), *300 Bd KW* (1600/1800 Hz in LSB/USB, z. B. Net105) und *9600 Bd G3RUH* (Basisband mit Scrambler, nur über den 9600-Bd-Datenanschluss des Funkgeräts).
- **Frequenzen** (Frequenz in der Funkgerät-Leiste antippen, stellt auch Mode und Geschwindigkeit ein): 2 m 144,925 und 144,950 MHz (Digitalbereich im Bandplan), 4 m 70,325 und 70,4875 MHz, 70 cm 433,625/433,650 MHz und 438,025–438,175 MHz (IARU-R1-Packetbereich, oft 9600 Bd), Kurzwelle 14,105 MHz LSB (Net105), 10,144 MHz LSB (Net30), 7,104 MHz LSB (Net40) und 3,5987 MHz LSB (Net80). Digipeater und Benutzereinstiege in der Nähe stehen in den Relaislisten.
- **Monitor:** jede Zeile wie bei einem TNC, `DL1ABC-7>DB0XYZ,DB0AAA* [I s2 r3 C]: Text`, mit Rahmenart (I, RR, RNR, REJ, SABM, UA, DISC, DM, FRMR, UI), Folgenummern, Poll/Final sowie Kommando (C) bzw. Antwort (R). Eigene Rahmen sind orange, Rahmen an oder von dir grün. Ein Klick übernimmt den Absender als Verbindungsziel.
- **Verbinden:** Ziel eintragen (`DB0XYZ` oder `DB0XYZ via DB0AAA DB0BBB`) und *Verbinden* drücken. Danach geht jede Eingabezeile (Enter) an die Gegenstelle; empfangener Text erscheint im Terminal. *Trennen* baut die Verbindung ab. Ohne Verbindung geht die Eingabe als Unproto-Text an das eingestellte Ziel (Standard `CQ`).
- **Protokoll:** AX.25 2.0 (modulo 8) mit Bestätigungen, Wiederholungen nach FRACK, Poll bei Zeitüberschreitung, REJ bei Lücken, Rückweg über die umgekehrten Digipeater. Eingehende Verbindungen werden angenommen (abschaltbar) und mit einem Begrüßungstext beantwortet (Makros wie `%m`, `%o`). Es gibt eine Verbindung zur Zeit; weitere Anrufer bekommen DM. Gesendet wird nur bei freiem Kanal (Belegt-Erkennung, zufällige Wartezeit wie p-persistent).
- **TNC-Parameter** (unter *Einstellungen* im rechten Bereich): SSID, PACLEN, MAXFRAME, FRACK, RETRIES, TXDELAY. Für Kurzwelle eher PACLEN 40–60, MAXFRAME 1, FRACK 5–8 s.
- Packet-Verbindungen landen nicht im Logbuch.
- **Geprüft:** Alle Rahmenarten, die Rustwave sendet, liest Direwolf (atest) in 1200, 300 und 9600 Bd, und es deutet Steuerfeld und Kommando/Antwort-Bits gleich. Von Direwolfs Testsignalen (gen_packets, 100 Pakete mit Rauschen) liest Rustwave bei 1200 Bd 74 (Direwolf 70), bei 9600 Bd 65 bzw. 57 (Direwolf 66 bzw. 62). Zwei Rustwave-Stationen übertragen Daten über einen Kanal mit bis zu 40 % Rahmenverlust vollständig und in der richtigen Reihenfolge. Mit einer echten Mailbox ist das Verbindungsprotokoll noch nicht getestet.

### Vorlagen
Ebenen-Editor nach dem Vorbild von MMSSTV:
- **Ebenen:** Text, Sendefoto, Empfangsbild (Bild im Bild), Bilddatei (auch per Drag & Drop) und Rechteck.
- **Text:** fünf Schriften, Größe, Ausrichtung, mehrzeilig, Füllung einfarbig oder als Verlauf, Rand, Schatten, Makro-Knöpfe.
- **Für jede Ebene:** Drehung, Perspektive (oben oder unten verjüngt), Deckkraft, sichtbar/unsichtbar, Reihenfolge.
- **Maus:** Klicken wählt eine Ebene, Ziehen verschiebt sie, der weiße Griff skaliert, der orange Griff dreht (mit Umschalt in 15°-Schritten). Pfeiltasten verschieben pixelweise, Entf löscht.
- **Phase:** Jede Vorlage kann einer Phase zugeordnet werden (CQ, Antwort, Rapport, 73), jede Phase gehört genau einer Vorlage.
- **Speichern:** Alles wird automatisch gespeichert (`%APPDATA%\Rustwave\vorlagen.json`). Positionen sind relativ, deshalb passt eine Vorlage in jedem SSTV-Modus.

**Makros:** `%c` Rufzeichen · `%n` Name · `%q` QTH · `%r` RSV gesendet · `%s` RSV empfangen · `%x` Notiz · `%m` eigenes Rufzeichen · `%o` eigener Name · `%a` eigener QTH · `%l` Locator · `%g` Gruß GM/GA/GE · `%t` Uhrzeit UTC · `%d` Datum · `%f` Frequenz · `%b` Band · `%v` SSTV-Modus · `%w` Anzahl früherer QSOs · `%%` Prozentzeichen

### Logbuch
- Alle QSOs (SSTV und RTTY) mit Suche. ✏ lädt ein QSO zum Bearbeiten in die QSO-Leiste (*Ins Log* aktualisiert es dann), 🗑 löscht es, Rückgängig ist möglich.
- **ADIF exportieren** schreibt eine ADIF-3.1-Datei (Modus SSTV bzw. RTTY) für Logprogramme, LoTW und eQSL. Umlaute werden dabei normgerecht als ae/oe/ue/ss geschrieben.
- Ablage: `%APPDATA%\Rustwave\logbuch.json`


## TNC für andere Programme (KISS/AGW)

*Einstellungen › TNC*: Rustwave arbeitet als Software-TNC wie Dire Wolf – **KISS über TCP** (Port 8001) und eine Teilmenge von **AGWPE** (Port 8000: Monitor, Rohrahmen, Unproto; Verbindungen über AGW meldet Rustwave sofort als getrennt, dafür KISS nehmen). Alle empfangenen Rahmen gehen an die verbundenen Programme; deren Rahmen sendet Rustwave über das Modem des offenen Reiters – APRS (1200 Bd FM, 300 Bd KW) oder Packet (300/1200/9600 Bd, mit Kanalzugriff). So funktionieren Winlink Express (Packet WL2K, TNC-Typ KISS, TCP 127.0.0.1:8001), YAAC, APRSIS32, Xastir, Outpost oder BPQ32 auch über das WLAN des IC-705. Standardmäßig nur vom eigenen PC erreichbar; „auch aus dem Netzwerk“ öffnet die Ports im Heimnetz. Gegengeprüft mit Dire Wolfs `kissutil` in beide Richtungen.

## Sprechfunk über WLAN/USB

🔊 in der Funkgerät-Leiste schaltet das **Mithören** ein (Empfang über den PC-Lautsprecher, bei ZF nach der FM-Demodulation); **Sprechen** (gedrückt halten) oder die **Leertaste** schaltet auf Senden und schickt das PC-Mikrofon zum Funkgerät – über WLAN (48 kHz in 20-ms-Paketen) bzw. über den USB-Audioausgang. Wahlweise schaltet die Taste um statt gehalten zu werden; nach 180 s geht Rustwave von selbst auf Empfang. Mikrofon, Pegel und Lautsprecher unter *Einstellungen › Audio*. Am IC-705 die Sprache vom PC freigeben: *SET › Connectors › MOD Input › DATA OFF MOD* auf WLAN bzw. USB.

## Satelliten und ISS

**Sat** in der Funkgerät-Leiste öffnet das Satellitenfenster: Bahndaten von CelesTrak (Amateurfunk- und Stationssatelliten), ersatzweise von AMSAT bzw. SatNOGS, oder aus einer Datei (als `tle.txt` im Rustwave-Ordner; Verbindungen zuerst über IPv4), Bahnberechnung mit SGP4 (gegen skyfield geprüft), Himmelskarte mit dem nächsten Überflug, Azimut/Elevation/Entfernung, Doppler für Down- und Uplink und die Überflüge der nächsten 24 h. **Doppler per CAT nachführen** korrigiert im Empfang die Downlink-, beim Senden die Uplink-Frequenz. Voreinstellungen für die ISS: APRS-Digipeater 145,825 MHz (Pfad ARISS), SSTV 145,800 MHz und Sprechfunk Region 1 (145,800/145,200 MHz); ihre Frequenzen lassen sich überschreiben (z. B. wenn die ISS für eine SSTV-Aktion ins 70-cm-Band ausweicht) und mit „Standard“ zurücksetzen. Andere Satelliten mit eigenen Frequenzen. Mit **bei Überflug Reiter und Frequenz einstellen** wechselt Rustwave beim Aufgang in den passenden Reiter, stellt FM und Frequenz ein (bei ISS-APRS den Pfad ARISS) und stellt danach den alten Pfad wieder her.

## Signalstärke

Bei Icom-Geräten fragt Rustwave im Empfang das S-Meter etwa viermal pro Sekunde ab, zeigt es in der Funkgerät-Leiste und merkt sich den Wert bei jedem dekodierten Paket: Spalte *Pegel* und Stationskarte im APRS-Reiter, Pegel je Sender in der Liste der gehörten DAPNET-Sender (letzter, Mittel, höchster).

## Funkgerät-Leiste: Tuner, Empfang, Versorgung, Uhr (Icom)
- **TUNE** startet den Antennentuner (IC-705 mit AH-705, IC-7300 mit eingebautem Tuner). Die Leiste zeigt währenddessen „stimmt ab …“ und SWR und Po live, danach steht das Ergebnis in der Statuszeile. Rechtsklick schaltet den Tuner ein/aus (hervorgehoben = eingeschaltet).
- **RX …** öffnet die Empfangsschalter: Filter FIL1–3, Vorverstärker, Abschwächer 20 dB, Störaustaster, Rauschminderung und AGC. Auf dem Knopf steht, was gerade aktiv ist, z. B. „RX FIL2 P1 NR“. Änderungen am Gerät erscheinen nach wenigen Sekunden.
- **Versorgungsspannung** (z. B. „7.4 V“), beim Senden auch der Strom. Unter 9 V nimmt das Programm Akkubetrieb an (BP-272): orange unter 7,0 V, rot unter 6,6 V mit Warnung in der Statuszeile. Bei Netzteilbetrieb: orange unter 12 V, rot unter 11,5 V.
- **Uhr ±N s** erscheint, wenn die PC-Uhr mehr als 2 s von der GPS-Zeit des Funkgeräts abweicht. Ein Klick darauf und dann *PC-Uhr nach GPS stellen* korrigiert sie. Windows fragt dabei nach Administratorrechten. Die GPS-Zeit des Geräts hat Sekundenauflösung, das reicht für Log, APRS und FT8.

## APRS: iGate, Unterhaltungen, Bulletins, Wetter

- **Stationskarte (Mouse-over):** Zeiger auf eine Zeile der Stationsliste bzw. auf eine Station in der Karte öffnet ein kleines Fenster mit Kompassrose (Fahrtrichtung grün, Wind blau, Richtung von dir aus orange), Position in Grad/Minuten, Dezimal und Locator, Höhe, Entfernung, Tempo und Kurs, Wetter (Temperatur, Feuchte, Luftdruck, Wind in km/h und Beaufort, Böen, Regen 1 h/24 h/seit Mitternacht, Sonne), PHG im Klartext mit Reichweite, Mic-E-Status (Unterwegs, Außer Dienst, Notfall …), Gerät (Mic-E-Kennung bzw. Zielrufzeichen, z. B. Kenwood TM-D710, Yaesu FTM-400, APRSdroid), Zeitstempel, Empfangsweg (direkt oder über welchen Digipeater) und dem Rohpaket. Die Kommentarspalte zeigt Kurs/Tempo lesbar statt `088/036/A=…`.
- **Wetter in der Stationskarte:** Wetterdaten werden aus allen Varianten gelesen – Position mit Wettersymbol (auch mit fehlenden Werten `.../...`), komprimierte Position (Wind steckt dann im Kurs/Tempo-Feld), positionsloser Wetterbericht `_MMDDhhmm…` und Wetterdaten ohne Wettersymbol – und erscheinen gegliedert statt im Kommentar: Temperatur mit gefühlter Temperatur (Windchill bzw. Hitzeindex) und Taupunkt, Luftfeuchte, Wind in km/h, m/s und Beaufort mit Bezeichnung, Böen, Richtung, Luftdruck mit Tendenz über 3 h (aus den empfangenen Berichten), Regen 1 h/seit Mitternacht/24 h, Neuschnee, Sonneneinstrahlung, Spannung, Regenzähler und die Kennung von Software und Wetterstation (z. B. „eCumulusDsVP“ = Cumulus, Davis Vantage Pro), dazu das Alter des letzten Berichts.
- **Spuren und Verläufe:** Rustwave merkt sich die Positionen mobiler Stationen (24 h) und zeichnet sie auf der Karte (*Spuren*); die Stationskarte nennt Punkte und Strecke. Wetterstationen bekommen kleine Kurven für Temperatur und Luftdruck (aus den empfangenen Berichten, Punkte im Abstand von mindestens 5 min).
- **Telemetrie:** Messwerte als `T#…` und Base91-Telemetrie im Kommentar (`|…|`) werden mit der Beschreibung der Station (Nachrichten `PARM.`, `UNIT.`, `EQNS.`, `BITS.`) umgerechnet und in der Stationskarte mit Namen und Einheiten gezeigt, Schaltbits im Klartext.
- **Funkwege:** Auf der Karte führen Linien von jeder auf Funk gehörten Station über die benutzten Digipeater (sofern deren Position bekannt ist) zu dir; blau = direkt gehört, orange = über Digipeater. Die Station unter dem Mauszeiger zeigt ihren Weg immer.
- **SmartBeaconing:** Voreinstellungen *Auto*, *Fahrrad* und *zu Fuß*; die Höhe vom GPS geht als `/A=` mit (abschaltbar); wahlweise wird die Bake im kompakten **Mic-E**-Format gesendet (Position, Kurs, Tempo und Höhe binär, Status „Unterwegs“ bzw. „Außer Dienst“) – *Einstellungen › APRS*.
- **Hinweise:** Meldung (Statuszeile, Liste, Blinken in der Taskleiste), wenn bestimmte Rufzeichen gehört werden (`*` als Platzhalter, ohne SSID gilt jede), eine Station in einen Umkreis kommt oder eine Wetterstation Böen, Regen oder Frost über einer Schwelle meldet – je Station und Anlass höchstens stündlich.
- **Objekte:** Eigene Objekte (Name bis 9 Zeichen, Position, Symbol, Kommentar) senden, mit der Bake bzw. alle 10 min wiederholen und wieder löschen. Empfangene Objekte zeigen, wer sie gesetzt hat; gelöschte verschwinden aus der Liste.
- **Abfragen:** `?APRSP` (Position), `?APRSS` (Status), `?APRSO` (Objekte), `?APRSD` (direkt gehörte Stationen), `?APRSH Rufzeichen` (gehört?), `?APRST`/`?PING?` (Weg des Pakets) per Nachricht sowie die allgemeinen Abfragen `?APRS?` und `?IGATE?` werden automatisch beantwortet (je Station höchstens einmal pro Minute, allgemeine nach Zufallszeit bis 2 min; abschaltbar).
- **Export:** Stationen mit Position und ihre Spuren als GPX (Navigationsgeräte, Karten-Apps) oder KML (Google Earth) in den Rustwave-Ordner.
- **iGate** (*Einstellungen › APRS › Internet*): Das Programm leitet auf Funk gehörte Pakete an APRS-IS weiter (Empfangs-iGate, „qAR“), sodass sie z. B. auf aprs.fi erscheinen. Aus dem Internet wird nichts auf Funk gesendet. Nicht weitergeleitet werden Pakete mit NOGATE, RFONLY oder TCPIP im Pfad, Abfragen und Drittverkehr. Alle 30 min geht die eigene iGate-Position (Symbol R&) ins Internet. Für ein iGate ist eine eigene SSID üblich, z. B. -10. Den Passcode berechnet das Programm selbst.
- **Stationen aus dem Internet** im einstellbaren Umkreis (braucht die eigene Position): Sie erscheinen blass in der Stationsliste und mit „[IS]“ im Monitor. Nachrichten an dich, die übers Internet kommen, erscheinen ebenfalls und werden dort bestätigt.
- **Wege für Nachrichten** (*Einstellungen › APRS › Nachrichten*, oder Klick auf „Empfang: … · Senden: …“ über den Nachrichten), getrennt für Empfang und Senden:
  - *Empfangen über* **Funk** (Standard), **Internet** oder **Funk + Internet**. Bei „Internet“ kommen Nachrichten von aprs.fi, Apps und Online-Diensten direkt über APRS-IS an, auch wenn kein iGate in deiner Nähe sie auf Funk aussendet; viele iGates empfangen nur. Nachrichten über einen abgewählten Weg werden ignoriert. Bestätigungen für deine eigenen Nachrichten zählen dagegen immer. Die Anmeldung filtert auf Nachrichten an dein Rufzeichen mit und ohne SSID.
  - *Senden über* **Automatisch** (Standard), **Funk**, **Internet** oder **Funk + Internet**. Das gilt für eigene Nachrichten und Bestätigungen. „Automatisch“ antwortet auf dem Weg, auf dem die Gegenstelle zuletzt kam: Internet-Gegenstellen übers Internet, Funkstationen auf Funk. Ist „Internet“ gewählt und gerade keine Verbindung da, wartet die Nachricht, statt auf Funk auszuweichen.
  - Jede Sprechblase zeigt, auf welchem Weg die Nachricht kam bzw. ging (Funk, iGate, Internet).
- **Rauschsperre:** Kommt im APRS-Betrieb länger als 20 s nur Stille vom Funkgerät, weist Rustwave darauf hin. Meist ist dann die Rauschsperre (SQL) zu, und schwache Pakete öffnen sie nicht. Der Knopf *Rauschsperre öffnen* stellt SQL über CAT auf 0. Der Dekoder kommt mit dem Rauschen zurecht.
- **Nachrichten aus dem Internet** erreichen dich auf drei Wegen (je nach eingestelltem Empfangsweg):
  - direkt über Funk,
  - über ein iGate in deiner Nähe, das sie als „Drittverkehr“ (Paket beginnt mit `}`) auf Funk aussendet; die Bestätigung geht auf Funk zurück und das iGate leitet sie weiter,
  - über die eigene APRS-IS-Verbindung.
  Angenommen werden Nachrichten an dein APRS-Rufzeichen mit SSID (z. B. DL4RE-9) und an das Grundrufzeichen ohne SSID (DL4RE). Viele Online-Dienste adressieren so und senden dabei sogar unter deinem eigenen Rufzeichen. Solche Pakete hält Rustwave nicht mehr für das Echo der eigenen Aussendung.
  Bestätigungen im neueren Reply-Ack-Format (`{MM}AA`) markieren deine eigenen Nachrichten ebenfalls als bestätigt.
- **Nachrichten als Unterhaltungen**: links die Gegenstationen (ungelesene grün mit Anzahl), rechts der Verlauf als Sprechblasen. Eigene stehen rechts und zeigen den Stand: gesendet 1/3 bzw. bestätigt.
- **Bulletins** (Rundsprüche an BLN…) über den Knopf *Bulletins* neben der Überschrift.
- **Wetterstationen**: Temperatur, Wind, Böen, Luftfeuchte, Luftdruck und Regen stehen lesbar in der Stationsliste statt der Rohdaten.

## SSTV-Galerie
Der Reiter **Galerie** zeigt alle empfangenen Bilder, das neueste zuerst. Filtern lässt sich nach Rufzeichen oder Modus (Suchfeld), Zeitraum (Heute, 7 Tage, 30 Tage, Alle), Modus und Band. Ein Klick zeigt das Bild groß mit Zeit, Modus, Frequenz und Rufzeichen (aus dem FSKID). Von dort lässt es sich als „Bild im Bild“ für die Antwort übernehmen. Zu jedem Bild speichert das Programm eine kleine .json-Datei daneben. Ältere Bilder ohne diese Datei zeigen Zeit und Modus aus dem Dateinamen.

**Automatisch kopieren** (*Einstellungen › SSTV*): Jedes empfangene Bild landet zusätzlich in einem Ordner deiner Wahl. Ist das ein OneDrive- oder Dropbox-Ordner oder eine Netzwerkfreigabe, werden die Bilder so automatisch hochgeladen.

## KW-APRS-Skimmer
Bei **KW 300 Bd** ist **Skimmer** eingeschaltet. Dann lauscht das Programm nicht nur um die Normallage (Tonmitte 1700 Hz ±100 Hz), sondern im ganzen Durchlassbereich (Tonmitte 500–2600 Hz, alle 30 Hz ein Dekoder). Es findet also auch Stationen, die neben der Normalfrequenz senden, und mehrere gleichzeitig.

- In der Stationsliste erscheint die Spalte **Frequenz**, im Monitor steht die Frequenz in eckigen Klammern vor jedem Paket. Ohne CAT steht dort die Tonlage in Hz.
- Liegt eine Station mehr als 100 Hz neben der Normallage, ist ihre Frequenz orange.
- Den Filter am Funkgerät breit lassen (2,4 kHz in USB).
- Gesendet wird weiter auf der eingestellten Frequenz mit 1600/1800 Hz.
- Rechenlast: etwa 3 % eines Kerns auf einem schnellen PC. Auf einem sparsamen Tablet-Prozessor ist es mehr, bei Bedarf den Skimmer abschalten.

## Tastenkürzel
| Taste | Wirkung |
|---|---|
| F1 … F4 | Betriebsart SSTV, RTTY, CW, APRS |
| F9 | Betriebsart POCSAG |
| F10 | Betriebsart Packet Radio |
| F5 … F8 | CQ, Antwort, Rapport, 73 senden (SSTV-Vorlage bzw. RTTY-/CW-Makro) |
| Strg+Enter | Senden: SSTV-Bild, freien Text (sonst das Makro der Phase), APRS-Nachricht bzw. Bake, Funkruf |
| Esc | Senden abbrechen, PTT lösen |
| Strg+L | QSO ins Log |
| Strg+, | Einstellungen |
| Strg+R | Aufnahme starten/beenden |

## Rechenlast
Rustwave zeichnet die Oberfläche nur neu, wenn sich etwas ändert, bei laufendem Empfang in einem festen Takt. Die **Bildrate** stellst du unter *Einstellungen › Darstellung* ein: sparsam 8/s, normal 15/s (Standard) oder flüssig 25/s. Die Dekoder arbeiten unabhängig davon immer mit voller Genauigkeit. **Energiesparen** setzt die Bildrate auf 8/s und den Wasserfall auf 10 Zeilen/s. Gemessen mit Audio und Spektrum über WLAN: früher rund 50 Bilder/s, jetzt mit „normal“ etwa ein Drittel der CPU-Last und mit „sparsam“ etwa ein Sechstel.

## Logo
Die Welle beginnt links als Sinuskurve (analog: Sprache, SSTV) und geht rechts in einen Rechteckpuls über (digital: RTTY, CW, APRS). Oben steht das Funksymbol, die Farbe ist Rost-Orange. Die Dateien liegen in `assets/logo/`: Symbol als SVG, PNG und ICO, dazu der Schriftzug für dunklen (`rustwave-logo.svg`) und hellen Hintergrund (`rustwave-logo-hell.svg`).

Empfangene Bilder, Aufnahmen und Selbsttest-Berichte liegen unter `Bilder\Rustwave`. Der Knopf 📁 neben ⏺ in der Funkgerät-Leiste öffnet den Ordner. Bilder und Aufnahmen aus dem alten Ordner `Bilder\SSTV` übernimmt Rustwave beim ersten Start. Einstellungen, Vorlagen, Profile und Logbuch liegen unter `%APPDATA%\Rustwave`. Beim ersten Start übernimmt Rustwave alles aus dem alten Ordner `%APPDATA%\SSTV Station` (es wird kopiert, die Statuszeile meldet das). Der alte Ordner bleibt liegen und kann danach gelöscht werden.

## Weiteres
- **Folgt dem Abstimmknopf:** Frequenz, Betriebsart und Sendeleistung des Funkgeräts werden laufend angezeigt. Ist am Gerät *CI-V Transceive* auf ON, kommt jede Änderung sofort, sonst etwa alle 0,5 s.
- **Frequenzgedächtnis je Betriebsart:** SSTV, RTTY, CW, APRS, POCSAG und Packet Radio merken sich jeweils ihre letzte Frequenz und Betriebsart. Beim Umschalten (z. B. SSTV → CW) stimmt das Funkgerät darauf ab.
- **Sendeleistung** (Funkgerät-Leiste): Bei Icom in Prozent wie im Menü RF POWER (beim IC-705 entsprechen 100 % etwa 10 W bei 13,8 V), bei Kenwood und Yaesu in Watt. Dreht man am Gerät, läuft der Regler mit.
- **Messwerte beim Senden** (Icom): Neben dem Regler erscheinen die tatsächliche Ausgangsleistung (Po in Watt), das SWR (grün unter 1,5, orange bis 2,5, darüber rot) und die ALC als Balken (rot, wenn sie über den Regelbereich geht). Nach dem Senden bleiben die letzten Werte noch 15 s blass stehen.
- **Spektrum vom Funkgerät** (*Ansicht › Spektrum vom Funkgerät*, Icom mit Scope wie IC-705/7300/9700): Kurve und Wasserfall über dem Betriebsbereich. Die rote Linie ist die Gerätefrequenz, ein Klick stimmt auf die Stelle ab. Zwei Ansichten:
  - **Band** (IC-705): zeigt das ganze Band (z. B. 40 m 7,000–7,200 MHz) und bleibt stehen, wenn du abstimmst. Gewechselt wird das Band über die Frequenz oben (Bandknöpfe oder Eingabe), das Spektrum zieht mit. **◀ ▶** verschiebt den Ausschnitt, **+ −** zoomt (10 kHz bis ganzes Band), **VFO** legt den Ausschnitt um die Gerätefrequenz. Liegt die Gerätefrequenz außerhalb, zeigt ein roter Pfeil am Rand die Richtung. Dafür stellt die App das Gerät auf den Festmodus und belegt dort die **feste Kante 3** des jeweiligen Bereichs mit dem Ausschnitt.
  - **Mitte**: Gerätefrequenz in der Mitte, **+ −** wählt die Spannweite (±2,5 bis ±500 kHz).
  - **IC-7300 und andere über USB:** Das Spektrum kommt nur mit 115200 Bd. Am Gerät dafür *MENU › SET › Connectors › CI-V*: CI-V USB Port = „Unlink from [REMOTE]“, CI-V USB Baud Rate = 115200 (oder Auto), und in Rustwave unter *Einstellungen › Transceiver* ebenfalls 115200 Bd. Kommen keine Daten, zeigt Rustwave nach wenigen Sekunden einen Hinweis mit Knopf zum Umstellen.
  - Das Gerät zeigt auf einmal höchstens etwa 1 MHz. Breitere Bänder wie 2 m (2 MHz) und 70 cm (10 MHz) siehst du deshalb in Ausschnitten, die du verschiebst. Lehnt das Gerät einen zu breiten Ausschnitt ab, merkt sich die App die Grenze und zeigt einen Hinweis.
- **CW-Skimmer** (Betriebsart CW): dekodiert bis zu 10 CW-Signale im Durchlassbereich gleichzeitig. Die Liste zeigt Frequenz, Tempo, Rufzeichen und den Text. Ein Klick aufs Rufzeichen übernimmt es ins QSO, *QSY* zieht das Signal auf die eingestellte Tonhöhe, und ein Klick auf die Zeile setzt den Hauptdecoder darauf. Dafür den Filter am Gerät breit lassen (z. B. 2,4 kHz in USB).
- **Profile** (*Einstellungen › Profile*, schnell laden über *Funkgerät › Profil laden*): alle Einstellungen unter einem Namen speichern, z. B. „Zuhause“ (IC-7300 per USB) und „Portabel“ (IC-705 als Access Point, Touch). Ein Klick lädt das Profil, trennt die alte Verbindung und verbindet wie im Profil gespeichert.
- **Selbsttest** (*Hilfe › Selbsttest*): prüft Audio (tatsächliche Abtastrate), Funkgerät mit Rohdaten, WLAN-Verluste, Access Point, GPS, Uhr, Windows-Standort und die Ablageordner. Der Bericht lässt sich kopieren oder speichern.
- **GPS-Zeit:** Liefert das Funkgerät GPS-Daten und geht die PC-Uhr um 2 s oder mehr falsch, nimmt das Logbuch die GPS-Zeit (Hinweis in der Statuszeile).
- **Energiesparen** (*Ansicht*): Wasserfall mit 10 statt 25 Zeilen pro Sekunde. Unabhängig davon wird nur noch neu gezeichnet, wenn sich etwas ändert, und der Wasserfall nur berechnet, wenn er zu sehen ist.
- **Kleine Bildschirme und Touch** (z. B. Toughbook FZ-M1, 1280×800): Die Einstellungen liegen in einem eigenen Fenster, der Betriebsbereich hat die volle Breite, und der Inhalt ist rollbar (auch per Wischen). Bei sehr schmalem Fenster schaltet man zwischen *Empfang* und *Senden* um. Bei niedrigem Bildschirm steht *Senden* direkt unter der Vorschau.
- **Touch-Bedienung:** Bei der ersten Berührung des Bildschirms werden Knöpfe, Abstände, Rollbalken und die Griffe im Vorlagen-Editor größer (abschaltbar unter *Ansicht*). Die Menüs liegen dann hinter **☰**, der Knopf **⌨** blendet die Windows-Bildschirmtastatur ein. Unter *Ansicht › Zoom* lässt sich alles vergrößern.
- **Aufnahme:** Der Knopf **⏺** in der Funkgerät-Leiste (oder Strg+R) schneidet das empfangene Audio als WAV mit (`%USERPROFILE%\Pictures\Rustwave\aufnahme-….wav`), z. B. um Signale zum Prüfen weiterzugeben. *Datei › Ordner mit Bildern und Aufnahmen öffnen* zeigt den Speicherort.
- Empfangene Bilder werden in `%USERPROFILE%\Pictures\Rustwave` gespeichert.
- WAV-Dateien lassen sich dekodieren, indem man sie ins Fenster oder auf die `.exe` zieht.
- Einstellungen werden alle paar Sekunden gespeichert. Audio und CAT starten beim nächsten Mal automatisch wieder, wenn sie beim Beenden liefen.
- Beim Schließen wird die PTT immer gelöst. Bei einem Absturz steht ein Protokoll in `%TEMP%\rustwave-absturz.txt`.

## Unterstützt
- **SSTV:** Martin M1/M2/M3/M4, Scottie S1/S2/DX, Robot 36/72, Robot 8/24 S/W, Wraase SC2-120/SC2-180, Pasokon P3/P5/P7, PD 50/90/120/160/180/240/290, jeweils TX und RX. Die Zeiten folgen der Modusliste von N7CXI/KB4YZ wie in slowrx, MMSSTV und QSSTV; Wraase, Pasokon und Robot S/W sind zusätzlich mit Aufnahmen aus PySSTV gegengeprüft.
- **Decoder:** VIS-Erkennung, Slant-Korrektur über die Sync-Impulse und Wiedereinrasten nach Audio-Aussetzern
- **RTTY:** Baudot 45,45/50/75 Baud · **CW:** Empfang 5–60 WpM · **APRS:** AFSK 1200 Baud (UKW) und 300 Baud (KW), AX.25 · **POCSAG:** 512/1200/2400 Baud, numerisch, alphanumerisch, Skyper, nur Ton (RX und TX) · **Packet Radio:** AX.25 2.0 mit Verbindungen, 1200 Bd AFSK, 300 Bd KW, 9600 Bd G3RUH
- **CAT:** Icom CI-V, Yaesu CAT (ASCII), Yaesu 5-Byte (FT-817/857/897), Kenwood/Elecraft sowie PTT über RTS/DTR oder VOX. Die Liste enthält über 30 vorkonfigurierte Geräte.

## IC-705 über WLAN (auch IC-9700, IC-7610, IC-905 über LAN)
CI-V **und** Audio laufen über das Netzwerk, ein USB-Kabel ist nicht nötig.
1. Am IC-705 unter `MENU > SET > WLAN Set` mit dem WLAN verbinden. Die IP-Adresse des Geräts steht in der Statusanzeige des WLAN-Menüs oder in der Geräteliste deines Routers.
2. `MENU > SET > WLAN Set > Remote Settings`: **Network Control = ON**, unter *Network User1* Benutzer und Passwort anlegen. Die UDP-Ports bleiben auf 50001/50002/50003.
3. `MENU > SET > Connectors > MOD Input`: **DATA OFF MOD = WLAN** (Senden in USB) bzw. **DATA MOD = WLAN** (Senden in USB-D). Die CI-V-Adresse bleibt `A4h`.
4. In der App unter *Einstellungen › Transceiver*: **WLAN / LAN** wählen, Gerät *Icom IC-705*, IP-Adresse, Benutzer und Passwort eintragen und auf *Verbinden* klicken.

Danach laufen Empfang und Senden über das Netz, die Funkgerät-Leiste zeigt „Audio WLAN“. Die PTT läuft immer über CI-V.
Das IC-705 erlaubt nur eine Netzwerkverbindung gleichzeitig: wfview, RS-BA1 oder SR5-Programme müssen vorher getrennt sein.
Das Protokoll ist von Icom nicht offengelegt. Die Umsetzung folgt kappanhang (MIT-Lizenz, HA2NON/ES1AKOS), die Paketaufbauten sind mit wfview abgeglichen.
Getestet und verifiziert mit einem echten IC-705 (Audio, CAT, PTT, GPS-Position über WLAN, SmartBeaconing); für automatische Tests gibt es zusätzlich ein simuliertes Funkgerät (`tools/fake_ic705.py`).

**Position ermitteln:** Für *vom Funkgerät* muss am IC-705 GPS eingeschaltet sein (MENU > GPS > GPS Set > GPS Select) und Satelliten empfangen werden. Für *Windows-Standort* unter Einstellungen > Datenschutz und Sicherheit > Standort die Standortdienste und den Zugriff für Desktop-Apps einschalten.

**Direkt ohne Router (IC-705 als Access Point):** Unter *Einstellungen › Transceiver › WLAN / LAN* auf **Direkt (Access Point)** umschalten. Das Funkgerät hat dann immer die Adresse 192.168.59.1. WLAN-Name und WLAN-Passwort des IC-705 eintragen und *PC mit IC-705-WLAN verbinden* drücken: Das Programm legt in Windows ein WLAN-Profil an, verbindet den PC und danach das Funkgerät. Ein grüner Haken zeigt, dass der PC im Funkgeräte-WLAN ist. Am IC-705: MENU > SET > WLAN Set: WLAN OFF › Connection Type: Access Point › Connection Settings (Access Point): SSID und Passwort festlegen › WLAN ON. Netzwerkbenutzer (Remote Settings) wie im Heimnetz. Es kann nur ein Gerät verbunden sein, und der PC hat währenddessen über WLAN kein Internet. Bekommt der PC keine Adresse, dem WLAN-Adapter eine feste Adresse geben (z. B. 192.168.59.10, Maske 255.255.255.0).

**Senden über WLAN:** Das Funkgerät puffert das Sendeaudio etwa 300 ms. Die PTT wird deshalb erst gelöst, wenn alles ausgespielt ist; vorher fehlte das Ende jeder Aussendung, bei APRS also die Prüfsumme.

**Verbindungsabbruch:** Reißt die WLAN-Verbindung ab, versucht das Programm nach 5, 10, 20 und dann alle 30 Sekunden neu zu verbinden (abbrechbar).

**WLAN-Audio:** Fehlende oder vertauschte Audiopakete werden beim Funkgerät neu angefordert und wieder in die richtige Reihenfolge gebracht. Im APRS-Modus zeigt eine Zeile, wie viele Pakete nachgefordert wurden und wie viele endgültig verloren gingen. Ist der Verlust hoch, hilft ein besserer WLAN-Empfang oder das USB-Kabel.

## IC-7300 (MENU > SET > Connectors)
- *USB SEND* = `OFF` und *USB Keying (CW/RTTY)* = `OFF`, wenn die PTT per CAT läuft
- *DATA OFF MOD* = `USB` (bzw. *DATA MOD* = `USB` für USB-D)
- *USB MOD Level* zusammen mit dem Sendepegel-Regler einstellen

## Selbst bauen
```
cargo test --release                       # Kern-, CAT-, Vorlagen-, Logbuch- und WAV-Tests
# Netzwerk Ende-zu-Ende gegen das simulierte IC-705 (rx.wav = Robot 36, 48 kHz mono):
python3 tools/fake_ic705.py --rx-wav /tmp/f/rx.wav --tx-wav /tmp/f/tx.wav --report /tmp/f/report.json &
SSTV_FAKE_DIR=/tmp/f cargo test --release gegen_simuliertes -- --ignored --nocapture
# POCSAG gegen multimon-ng: Aussendungen als Rohdatei (22050 Hz, s16le) schreiben und dort dekodieren
POCSAG_RAW=/tmp/p.raw cargo test --release export_raw -- --ignored
multimon-ng -t raw -a POCSAG512 -a POCSAG1200 -a POCSAG2400 /tmp/p.raw
# Packet Radio gegen Direwolf: alle Rahmenarten als WAV schreiben und mit atest lesen
PACKET_WAV=/tmp/pk.wav PACKET_SPEED=9600 cargo test --release export_packet -- --ignored
atest -B 9600 /tmp/pk.wav
cargo build --release                      # für das eigene System
cargo build --release --target x86_64-pc-windows-gnu   # Windows (mit mingw-w64)
```
Aufbau: `src/sstv.rs` (Modi, Encoder, Decoder), `src/rtty.rs` (Baudot, AFSK, Dekoder), `src/rtty_ui.rs` (RTTY-Oberfläche), `src/pocsag.rs` (POCSAG-Dekoder und -Kodierer), `src/pocsag_ui.rs` (POCSAG-Oberfläche), `src/ax25.rs` (AX.25-Rahmen, Verbindungsprotokoll, 9600-Bd-Modem), `src/packet_ui.rs` (Packet-Oberfläche), `src/icomnet.rs` (Icom-Netzwerkprotokoll), `src/template.rs` (Vorlagen, Makros, Renderer), `src/logbook.rs` (Logbuch, ADIF), `src/cat.rs` (Protokolle, serielle Schnittstelle), `src/audio.rs` (cpal/WASAPI), `src/editor.rs` (Vorlagen-Editor), `src/main.rs` (Oberfläche mit egui).

Netzwerkprotokoll nach kappanhang, MIT-Lizenz (siehe `../assets/kappanhang-LICENSE.txt`). Schriften: DejaVu Sans, Sans Bold, Sans Condensed Bold, Serif Bold, Sans Mono Bold (freie Lizenz, siehe `../assets/DejaVu-LICENSE.txt`).

---
© 2026 Roman Ewert, DL4RE · GPL-3.0-or-later
