//! ZF-Ausgabe der Icom-Geräte per CI-V umschalten: Für FM-Datenbetrieb (POCSAG, APRS UKW, Packet 1200/9600 FM) liefert
//! die 12-kHz-ZF flaches Basisband ohne Deemphasis und CTCSS-Hochpass; für SSB-Betriebsarten braucht es NF.
//! Menünummern (1A 05 …) nach den CI-V-Referenzen von Icom: Output Select 00 = AF, 01 = IF.

use super::*;

const TAG_IF: u8 = 20;

/// Menünummer für „Output Select“ (USB bzw. WLAN/LAN) je Gerät
pub(crate) fn items(model: &cat::RigModel) -> Option<([u8; 2], Option<[u8; 2]>)> {
    match model.name {
        "Icom IC-705" => Some(([0x01, 0x09], Some([0x01, 0x14]))),  // USB AF/IF, WLAN AF/IF
        "Icom IC-7300" => Some(([0x00, 0x59], None)),               // ACC/USB Output Select
        "Icom IC-9700" => Some(([0x01, 0x05], Some([0x01, 0x10]))), // USB AF/IF, LAN AF/IF
        _ => None,
    }
}

impl App {
    /// soll die ZF für die aktuelle Betriebsart an sein?
    pub(crate) fn want_if(&self) -> bool {
        match self.op {
            Op::Pocsag => true,
            Op::Aprs => !self.aprs.hf,
            Op::Packet => self.packet.speed != ax25::Speed::B300,
            _ => false,
        }
    }
    /// Befehl für den Ausgang, über den gerade verbunden ist
    fn if_cmd(&self, on: bool) -> Option<Vec<u8>> {
        let r = self.rig.as_ref()?;
        let (usb, net) = items(&r.model)?;
        let item = if r.is_net() { net? } else { usb };
        Some(cat::civ(&r.model, &[0x1A, 0x05, item[0], item[1], on as u8]))
    }
    /// wird ZF gerade automatisch geführt (Schalter an, passendes Gerät per CAT verbunden)?
    pub(crate) fn if_managed(&self) -> bool { self.if_auto && self.if_cmd(false).is_some() }

    /// je Bild: ZF am Gerät der Betriebsart anpassen
    pub(crate) fn if_tick(&mut self) {
        if self.rig.is_none() { self.if_sent = None; return; }
        if !self.if_managed() { return; }
        if self.poll.cmd_ng.contains(&TAG_IF) {
            self.poll.cmd_ng.retain(|&t| t != TAG_IF);
            self.if_auto = false;
            self.log("Das Funkgerät lehnt das Umschalten von AF/IF ab (Firmware?) – Automatik aus, ZF bitte am Gerät wählen");
            return;
        }
        let want = self.want_if();
        self.if_rx = want;
        if self.if_sent == Some(want) { return; }
        if let Some(c) = self.if_cmd(want) {
            self.poll.cmdq.push_back((TAG_IF, c));
            self.if_sent = Some(want);
            self.if_switched = Some(Instant::now());
            let via = if self.rig.as_ref().is_some_and(|r| r.is_net()) { "WLAN/LAN" } else { "USB" };
            self.log(format!("Funkgerät-Ausgang ({via}) auf {} gestellt", if want { "ZF (12 kHz) – FM wird in Rustwave demoduliert" } else { "NF" }));
        }
    }
    /// vor dem Trennen bzw. Beenden: wieder NF, damit andere Programme wie gewohnt arbeiten
    pub(crate) fn if_restore(&mut self) {
        if self.if_sent == Some(true) {
            if let Some(c) = self.if_cmd(false) { if let Some(r) = &mut self.rig { let _ = r.send_raw(&c); } }
            std::thread::sleep(Duration::from_millis(80));
            self.if_rx = false; // das Gerät liefert jetzt wieder NF
        }
        self.if_sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rig(n: &str) -> cat::RigModel { cat::RIGS.iter().find(|r| r.name == n).unwrap().clone() }
    #[test]
    fn befehle() {
        let m = rig("Icom IC-705");
        let (usb, net) = items(&m).unwrap();
        assert_eq!(cat::civ(&m, &[0x1A, 0x05, usb[0], usb[1], 1]), vec![0xFE, 0xFE, 0xA4, 0xE0, 0x1A, 0x05, 0x01, 0x09, 0x01, 0xFD]);
        assert_eq!(net, Some([0x01, 0x14]));
        let m = rig("Icom IC-7300");
        assert_eq!(cat::civ(&m, &[0x1A, 0x05, 0x00, 0x59, 0]), vec![0xFE, 0xFE, 0x94, 0xE0, 0x1A, 0x05, 0x00, 0x59, 0x00, 0xFD]);
        assert_eq!(items(&m).unwrap().1, None);
        assert!(items(&rig("Icom IC-9700")).is_some());
    }
}
