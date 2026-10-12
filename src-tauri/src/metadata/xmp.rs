//! XMP: the XML packet that holds titles, keywords, ratings, edit history and, sometimes, places
//! and previews.
//!
//! XMP is not rewritten here. It is XML under namespaces whose prefixes are whatever the writer
//! chose, and a partial edit that missed a spelling would leave the thing it was meant to remove.
//! So the question asked of a packet is only whether it names a place, an owner or a serial
//! number, or carries a preview of the picture, and the answer decides whether it travels whole
//! or not at all. The markers are matched on local names (`:City`), which survive a renamed
//! prefix.

/// A packet larger than this is not carried: JPEG holds 64 KiB per segment, and splitting a
/// packet across "extended XMP" segments is not implemented.
pub const MAX_XMP_BYTES: usize = 1 << 20;

/// Local names that name a place.
const PLACE: [&str; 11] = [
    "GPS",
    ":City",
    ":State",
    ":Country",
    ":CountryCode",
    ":Location",
    ":Sublocation",
    ":ProvinceState",
    ":LocationShown",
    ":LocationCreated",
    ":CountryName",
];

/// Local names that identify an owner or a device.
const IDENTITY: [&str; 5] = [
    ":SerialNumber",
    ":BodySerialNumber",
    ":LensSerialNumber",
    ":OwnerName",
    ":CameraOwnerName",
];

/// A preview of the picture, as it was when the packet was written.
const PREVIEW: [&str; 2] = ["xmpGImg", ":Thumbnails"];

/// What a packet was found to carry that a private export would not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Findings {
    pub place: bool,
    pub identity: bool,
    pub preview: bool,
    /// The bytes are not UTF-8, so nothing can be said about them.
    pub unreadable: bool,
}

impl Findings {
    pub fn any(&self) -> bool {
        self.place || self.identity || self.preview || self.unreadable
    }
}

pub fn inspect(packet: &[u8]) -> Findings {
    let Ok(text) = std::str::from_utf8(packet) else {
        return Findings {
            unreadable: true,
            ..Findings::default()
        };
    };
    let any = |markers: &[&str]| markers.iter().any(|marker| text.contains(marker));
    Findings {
        place: any(&PLACE),
        identity: any(&IDENTITY),
        preview: any(&PREVIEW),
        unreadable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(body: &str) -> Vec<u8> {
        format!(
            "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
             <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">{body}</rdf:RDF></x:xmpmeta>"
        )
        .into_bytes()
    }

    #[test]
    fn a_packet_with_only_a_title_and_keywords_is_clean() {
        let xmp = packet(
            "<rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Harbour at dusk</dc:title></rdf:Description>",
        );
        assert!(!inspect(&xmp).any());
    }

    #[test]
    fn places_are_found_under_any_prefix() {
        for body in [
            "<rdf:Description xmlns:exif=\"http://ns.adobe.com/exif/1.0/\" exif:GPSLatitude=\"49,17.2N\"/>",
            "<rdf:Description xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\"><photoshop:City>Vancouver</photoshop:City></rdf:Description>",
            "<rdf:Description xmlns:p=\"http://ns.adobe.com/photoshop/1.0/\" p:Country=\"Canada\"/>",
            "<rdf:Description xmlns:i=\"http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/\" i:Location=\"Pier\"/>",
        ] {
            let found = inspect(&packet(body));
            assert!(found.place, "{body}");
        }
    }

    #[test]
    fn identity_and_previews_are_found() {
        let serial = packet("<rdf:Description xmlns:aux=\"http://ns.adobe.com/exif/1.0/aux/\" aux:SerialNumber=\"123\"/>");
        assert!(inspect(&serial).identity);
        let preview = packet("<xmp:Thumbnails><rdf:Alt><rdf:li><xmpGImg:image>/9j/4AAQ</xmpGImg:image></rdf:li></rdf:Alt></xmp:Thumbnails>");
        assert!(inspect(&preview).preview);
    }

    #[test]
    fn bytes_that_are_not_text_are_unreadable() {
        let found = inspect(&[0xFF, 0xFE, 0x3C, 0x00]);
        assert!(found.unreadable && found.any());
    }
}
