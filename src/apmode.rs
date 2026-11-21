//! IC-705 als WLAN-Access-Point: der PC verbindet sich direkt mit dem WLAN des Funkgeräts.
//! Das Funkgerät hat dort immer die Adresse 192.168.59.1 (fest, laut Icom nicht änderbar).

use std::net::{Ipv4Addr, UdpSocket};

pub const AP_IP: &str = "192.168.59.1";

/// Welche eigene Adresse würde der PC zum Funkgerät nehmen? (verbindet nur den Socket, sendet nichts)
pub fn local_addr_towards(ip: &str) -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect((ip, crate::icomnet::CONTROL_PORT)).ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(a) if !a.is_unspecified() => Some(a),
        _ => None,
    }
}
/// Liegt die Adresse im Netz des Funkgeräts (192.168.59.0/24)?
pub fn in_ap_net(a: Ipv4Addr) -> bool { a.octets()[..3] == [192, 168, 59] }

/// Ist der PC gerade im WLAN des Funkgeräts? Liefert die eigene Adresse.
pub fn pc_in_ap_net() -> Option<Ipv4Addr> { local_addr_towards(AP_IP).filter(|a| in_ap_net(*a)) }

#[cfg_attr(not(windows), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}
/// Windows-WLAN-Profil (WPA2-Personal; ohne Passwort offen)
#[cfg_attr(not(windows), allow(dead_code))]
pub fn profile_xml(ssid: &str, pass: &str) -> String {
    let n = xml_escape(ssid);
    let security = if pass.is_empty() {
        "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>".to_string()
    } else {
        format!("<authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>\
                 <sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", xml_escape(pass))
    };
    format!("<?xml version=\"1.0\"?>\n<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
             <name>{n}</name><SSIDConfig><SSID><name>{n}</name></SSID></SSIDConfig><connectionType>ESS</connectionType>\
             <connectionMode>manual</connectionMode><MSM><security>{security}</security></MSM></WLANProfile>")
}

/// PC mit dem WLAN des Funkgeräts verbinden (Windows, über netsh) und warten, bis eine Adresse im Netz anliegt
#[cfg(windows)]
pub fn join_wlan(ssid: &str, pass: &str) -> Result<Ipv4Addr, String> {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    // netsh mit Zeitlimit: hängt es (z. B. ohne WLAN-Dienst), soll das Programm nicht ewig warten
    let netsh = |args: &[&str]| -> Result<(), String> {
        let out = std::env::temp_dir().join("rustwave-netsh.txt");
        let f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        let mut c = std::process::Command::new("netsh").args(args).creation_flags(NO_WINDOW)
            .stdin(std::process::Stdio::null()).stdout(f).spawn().map_err(|e| e.to_string())?;
        let t0 = std::time::Instant::now();
        let status = loop {
            if let Some(st) = c.try_wait().map_err(|e| e.to_string())? { break st; }
            if t0.elapsed() > std::time::Duration::from_secs(15) {
                let _ = c.kill();
                return Err(format!("netsh {} antwortet nicht", args[..2].join(" ")));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let msg = String::from_utf8_lossy(&std::fs::read(&out).unwrap_or_default()).trim().to_string();
        let _ = std::fs::remove_file(&out);
        if status.success() { Ok(()) } else { Err(if msg.is_empty() { format!("netsh {}: Fehler", args[..2].join(" ")) } else { msg }) }
    };
    // Profil anlegen (Datei enthält das Passwort: sofort wieder löschen)
    let path = std::env::temp_dir().join("rustwave-ic705-wlan.xml");
    std::fs::write(&path, profile_xml(ssid, pass)).map_err(|e| e.to_string())?;
    let r = netsh(&["wlan", "add", "profile", &format!("filename={}", path.display()), "user=current"]);
    let _ = std::fs::remove_file(&path);
    r?;
    netsh(&["wlan", "connect", &format!("name={ssid}"), &format!("ssid={ssid}")])?;
    let t0 = std::time::Instant::now();
    while t0.elapsed() < std::time::Duration::from_secs(25) {
        if let Some(a) = pc_in_ap_net() { return Ok(a); }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(format!("mit „{ssid}“ verbunden, aber keine Adresse 192.168.59.x erhalten – ggf. dem WLAN-Adapter eine feste Adresse geben \
                 (z. B. 192.168.59.10, Maske 255.255.255.0)"))
}
#[cfg(not(windows))]
pub fn join_wlan(_ssid: &str, _pass: &str) -> Result<Ipv4Addr, String> {
    pc_in_ap_net().ok_or_else(|| "bitte den PC von Hand mit dem WLAN des Funkgeräts verbinden".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profil_und_netz() {
        let x = profile_xml("IC-705 <Test>", "geh&eim\"1");
        assert!(x.contains("<name>IC-705 &lt;Test&gt;</name>") && x.contains("<keyMaterial>geh&amp;eim&quot;1</keyMaterial>") && x.contains("WPA2PSK"));
        assert!(profile_xml("X", "").contains("<authentication>open</authentication>") && !profile_xml("X", "").contains("keyMaterial"));
        assert!(in_ap_net("192.168.59.23".parse().unwrap()) && !in_ap_net("192.168.5.23".parse().unwrap()));
        // Weg zu einer Adresse im eigenen Rechner: Quelladresse ist Loopback, also nicht im Funkgeräte-Netz
        assert_eq!(local_addr_towards("127.0.0.1"), Some(Ipv4Addr::LOCALHOST));
    }
}
