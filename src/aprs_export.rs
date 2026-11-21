//! Gehörte APRS-Stationen und ihre Spuren als GPX (Navigationsgeräte, Karten-Apps) oder KML (Google Earth) speichern.

/// eine Station für den Export
pub struct Item<'a> {
    pub call: &'a str,
    pub pos: (f64, f64),
    pub last: i64,
    pub desc: String,
    /// Spur: (Zeit, Breite, Länge)
    pub track: &'a [(i64, f64, f64)],
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }
fn iso(t: i64) -> String { chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string() }

pub fn gpx(items: &[Item]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"Rustwave\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n");
    for it in items {
        s += &format!("  <wpt lat=\"{:.6}\" lon=\"{:.6}\"><time>{}</time><name>{}</name><desc>{}</desc></wpt>\n",
            it.pos.0, it.pos.1, iso(it.last), esc(it.call), esc(&it.desc));
    }
    for it in items.iter().filter(|i| i.track.len() >= 2) {
        s += &format!("  <trk><name>{}</name><trkseg>\n", esc(it.call));
        for (t, la, lo) in it.track { s += &format!("    <trkpt lat=\"{la:.6}\" lon=\"{lo:.6}\"><time>{}</time></trkpt>\n", iso(*t)); }
        s += "  </trkseg></trk>\n";
    }
    s + "</gpx>\n"
}

pub fn kml(items: &[Item]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document><name>Rustwave APRS</name>\n");
    for it in items {
        s += &format!("<Placemark><name>{}</name><description>{}</description><TimeStamp><when>{}</when></TimeStamp><Point><coordinates>{:.6},{:.6},0</coordinates></Point></Placemark>\n",
            esc(it.call), esc(&it.desc), iso(it.last), it.pos.1, it.pos.0);
        if it.track.len() >= 2 {
            let c: Vec<String> = it.track.iter().map(|(_, la, lo)| format!("{lo:.6},{la:.6},0")).collect();
            s += &format!("<Placemark><name>{} Spur</name><LineString><tessellate>1</tessellate><coordinates>{}</coordinates></LineString></Placemark>\n",
                esc(it.call), c.join(" "));
        }
    }
    s + "</Document></kml>\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formate() {
        let tr = [(1_700_000_000, 52.5, 13.4), (1_700_000_060, 52.51, 13.41)];
        let it = [Item { call: "DL1ABC-9", pos: (52.51, 13.41), last: 1_700_000_060, desc: "Auto <mobil> & Co".into(), track: &tr }];
        let g = gpx(&it);
        assert!(g.contains("<wpt lat=\"52.510000\" lon=\"13.410000\"><time>2023-11-14T22:14:20Z</time><name>DL1ABC-9</name><desc>Auto &lt;mobil&gt; &amp; Co</desc>"));
        assert_eq!(g.matches("<trkpt").count(), 2);
        let k = kml(&it);
        assert!(k.contains("<coordinates>13.410000,52.510000,0</coordinates>"));
        assert!(k.contains("13.400000,52.500000,0 13.410000,52.510000,0"));
    }
}
