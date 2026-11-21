# Mitmachen

Rückmeldungen, Fehlerberichte und Verbesserungen sind willkommen.

## Gerätetests

Rustwave ist bisher nur mit dem **Icom IC-705** (WLAN und USB) und dem **Icom IC-7300** (USB) getestet und verifiziert. Wer ein anderes Gerät aus der Liste nutzt, hilft sehr mit einem kurzen Bericht über die Vorlage [Gerätetest](.github/ISSUE_TEMPLATE/geraetetest.md): Was funktioniert (Frequenz, Mode, PTT, Audio, Spektrum, GPS), was nicht?

## Fehler melden

Bitte angeben:
- Rustwave-Version (*Hilfe › Über Rustwave*) und Betriebssystem
- Funkgerät und Verbindung (USB/seriell, WLAN Heimnetz, Access Point)
- Was du getan hast, was passiert ist und was du erwartet hast
- Den Bericht aus *Hilfe › Selbsttest* (Zugangsdaten werden dort nicht ausgegeben)
- Bei Dekodierproblemen eine kurze Aufnahme (⏺ in der Funkgerät-Leiste, WAV)

## Änderungen am Code

1. Vor dem Pull Request `cargo test --release` laufen lassen; neue Funktionen möglichst mit Test.
2. Oberfläche, Kommentare und Dokumentation sind auf Deutsch; Bezeichner im Code ebenso, wo es schon so ist.
3. Keine neuen Laufzeit-Abhängigkeiten, die das Programm über eine einzelne `.exe` hinaus vergrößern (keine DLLs, kein Installer).
4. Beiträge stehen unter derselben Lizenz wie das Projekt (GPL-3.0-or-later).

Für Arbeiten ohne Funkgerät: `tools/fake_ic705.py` simuliert ein IC-705 im Netzwerk, die `--ignored`-Tests in [docs/BAUEN.md](docs/BAUEN.md) erzeugen Testsignale.
