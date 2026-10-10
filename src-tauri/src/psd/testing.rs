//! A writer for just enough of the PSD/PSB format to test the reader with. Test-only.
//!
//! It is written from the same specification as the reader, so agreement between the two
//! shows consistency with that reading of the specification, not that real Photoshop files
//! parse. The tests say so where it matters.
/// top, left, bottom, right
pub type RectTuple = (i32, i32, i32, i32);

#[derive(Clone)]
pub struct TestLayer {
    pub name: String,
    pub rect: RectTuple,
    pub blend: [u8; 4],
    pub opacity: u8,
    pub flags: u8,
    pub clipped: bool,
    /// (channel id, planar samples, big-endian)
    pub channels: Vec<(i16, Vec<u8>)>,
    /// 0 raw, 1 RLE, 2 ZIP, 3 ZIP with prediction
    pub compression: u16,
    /// A section divider type (1 open folder, 2 closed folder, 3 bounding section).
    pub divider: Option<u32>,
    pub divider_blend: Option<[u8; 4]>,
    /// rect, default colour, flags
    pub mask: Option<(RectTuple, u8, u8)>,
    pub unicode_name: Option<String>,
    pub extra_blocks: Vec<([u8; 4], Vec<u8>)>,
    pub layer_id: Option<u32>,
}

impl TestLayer {
    pub fn pixel(name: &str, rect: RectTuple, samples: Vec<Vec<u8>>) -> Self {
        let ids = [0_i16, 1, 2, -1];
        Self {
            name: name.to_string(),
            rect,
            blend: *b"norm",
            opacity: 255,
            flags: 0,
            clipped: false,
            channels: samples
                .into_iter()
                .enumerate()
                .map(|(index, data)| (ids[index], data))
                .collect(),
            compression: 0,
            divider: None,
            divider_blend: None,
            mask: None,
            unicode_name: None,
            extra_blocks: Vec::new(),
            layer_id: None,
        }
    }

    pub fn divider(name: &str, kind: u32) -> Self {
        let mut layer = Self::pixel(name, (0, 0, 0, 0), Vec::new());
        layer.divider = Some(kind);
        layer.divider_blend = Some(*b"norm");
        layer
    }
}

pub struct TestDoc {
    pub version: u16,
    pub width: u32,
    pub height: u32,
    pub depth: u16,
    pub mode: u16,
    pub channels: u16,
    pub layers: Vec<TestLayer>,
    pub icc: Option<Vec<u8>>,
    pub resolution_ppi: Option<f32>,
    /// Put the layers in this document-level block instead of the layer info section
    /// (`Lr16` / `Lr32`), as Photoshop does for 16- and 32-bit files.
    pub layers_in_block: Option<[u8; 4]>,
    pub merged: Vec<u8>,
}

impl TestDoc {
    pub fn rgb8(width: u32, height: u32, layers: Vec<TestLayer>) -> Self {
        Self {
            version: 1,
            width,
            height,
            depth: 8,
            mode: 3,
            channels: 3,
            layers,
            icc: None,
            resolution_ppi: None,
            layers_in_block: None,
            merged: vec![0; (width * height * 3) as usize],
        }
    }
}

fn big(version: u16) -> bool {
    version == 2
}

fn put_len(out: &mut Vec<u8>, value: u64, wide: bool) {
    if wide {
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.extend_from_slice(&(value as u32).to_be_bytes());
    }
}

pub fn pack_bits_row(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < row.len() {
        let mut run = 1;
        while at + run < row.len() && row[at + run] == row[at] && run < 128 {
            run += 1;
        }
        if run >= 3 {
            out.push((1 - run as i16) as i8 as u8);
            out.push(row[at]);
            at += run;
        } else {
            let start = at;
            let mut count = 0;
            while at < row.len() && count < 128 {
                // Stop a literal run where a worthwhile repeat begins.
                if at + 2 < row.len() && row[at] == row[at + 1] && row[at] == row[at + 2] {
                    break;
                }
                at += 1;
                count += 1;
            }
            out.push((count - 1) as u8);
            out.extend_from_slice(&row[start..start + count]);
        }
    }
    out
}

/// One channel's data with its compression field, for a `width` x `height` channel.
pub fn encode_channel(
    samples: &[u8],
    width: usize,
    height: usize,
    depth: u16,
    compression: u16,
    version: u16,
) -> Vec<u8> {
    let row = (width * usize::from(depth)).div_ceil(8);
    let mut out = compression.to_be_bytes().to_vec();
    match compression {
        0 => out.extend_from_slice(samples),
        1 => {
            let mut rows = Vec::new();
            let mut counts = Vec::new();
            for line in samples.chunks(row.max(1)).take(height) {
                let packed = pack_bits_row(line);
                counts.push(packed.len());
                rows.push(packed);
            }
            for count in counts {
                if big(version) {
                    out.extend_from_slice(&(count as u32).to_be_bytes());
                } else {
                    out.extend_from_slice(&(count as u16).to_be_bytes());
                }
            }
            for packed in rows {
                out.extend_from_slice(&packed);
            }
        }
        2 | 3 => {
            let mut data = samples.to_vec();
            if compression == 3 {
                for line in data.chunks_mut(row.max(1)) {
                    match depth {
                        8 => {
                            for index in (1..line.len()).rev() {
                                line[index] = line[index].wrapping_sub(line[index - 1]);
                            }
                        }
                        16 => {
                            let mut values: Vec<u16> = line
                                .chunks_exact(2)
                                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                                .collect();
                            for index in (1..values.len()).rev() {
                                values[index] = values[index].wrapping_sub(values[index - 1]);
                            }
                            for (pair, value) in line.chunks_exact_mut(2).zip(values) {
                                pair.copy_from_slice(&value.to_be_bytes());
                            }
                        }
                        32 => {
                            // Gather the bytes of each sample into planes, then difference
                            // byte-wise across the row.
                            let width = line.len() / 4;
                            let mut planes = vec![0_u8; line.len()];
                            for x in 0..width {
                                for plane in 0..4 {
                                    planes[plane * width + x] = line[x * 4 + plane];
                                }
                            }
                            for index in (1..planes.len()).rev() {
                                planes[index] = planes[index].wrapping_sub(planes[index - 1]);
                            }
                            line.copy_from_slice(&planes);
                        }
                        _ => panic!("the test writer predicts only 8, 16 and 32 bits"),
                    }
                }
            }
            out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&data, 6));
        }
        _ => panic!("unknown compression"),
    }
    out
}

fn block(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8], wide_length: bool, pad_to: usize) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(key);
    put_len(out, data.len() as u64, wide_length);
    out.extend_from_slice(data);
    if pad_to > 1 {
        while !out.len().is_multiple_of(pad_to) {
            out.push(0);
        }
    }
}

fn is_long(key: &[u8; 4]) -> bool {
    matches!(key, b"Lr16" | b"Lr32" | b"Layr")
}

fn layer_records_and_data(doc: &TestDoc) -> Vec<u8> {
    let wide = big(doc.version);
    let mut records = Vec::new();
    let mut data = Vec::new();
    for layer in &doc.layers {
        let (top, left, bottom, right) = layer.rect;
        let width = (right - left).max(0) as usize;
        let height = (bottom - top).max(0) as usize;
        for value in [top, left, bottom, right] {
            records.extend_from_slice(&value.to_be_bytes());
        }
        records.extend_from_slice(&(layer.channels.len() as u16).to_be_bytes());
        let mut encoded = Vec::new();
        for (id, samples) in &layer.channels {
            let bytes = encode_channel(
                samples,
                width,
                height,
                doc.depth,
                layer.compression,
                doc.version,
            );
            records.extend_from_slice(&id.to_be_bytes());
            put_len(&mut records, bytes.len() as u64, wide);
            encoded.push(bytes);
        }
        records.extend_from_slice(b"8BIM");
        records.extend_from_slice(&layer.blend);
        records.push(layer.opacity);
        records.push(u8::from(layer.clipped));
        records.push(layer.flags);
        records.push(0);

        let mut extra = Vec::new();
        // Layer mask data.
        match &layer.mask {
            None => extra.extend_from_slice(&0_u32.to_be_bytes()),
            Some(((top, left, bottom, right), default, flags)) => {
                extra.extend_from_slice(&20_u32.to_be_bytes());
                for value in [top, left, bottom, right] {
                    extra.extend_from_slice(&value.to_be_bytes());
                }
                extra.push(*default);
                extra.push(*flags);
                extra.extend_from_slice(&[0, 0]);
            }
        }
        // Blending ranges: none.
        extra.extend_from_slice(&0_u32.to_be_bytes());
        // Name, Pascal, padded to 4.
        let name = layer.name.as_bytes();
        extra.push(name.len() as u8);
        extra.extend_from_slice(name);
        let mut written = 1 + name.len();
        while !written.is_multiple_of(4) {
            extra.push(0);
            written += 1;
        }
        if let Some(unicode) = &layer.unicode_name {
            let mut body = (unicode.encode_utf16().count() as u32)
                .to_be_bytes()
                .to_vec();
            for unit in unicode.encode_utf16() {
                body.extend_from_slice(&unit.to_be_bytes());
            }
            body.extend_from_slice(&[0, 0]);
            block(&mut extra, b"luni", &body, false, 4);
        }
        if let Some(kind) = layer.divider {
            let mut body = kind.to_be_bytes().to_vec();
            if let Some(blend) = layer.divider_blend {
                body.extend_from_slice(b"8BIM");
                body.extend_from_slice(&blend);
            }
            block(&mut extra, b"lsct", &body, false, 4);
        }
        if let Some(id) = layer.layer_id {
            block(&mut extra, b"lyid", &id.to_be_bytes(), false, 4);
        }
        for (key, body) in &layer.extra_blocks {
            block(&mut extra, key, body, wide && is_long(key), 4);
        }
        records.extend_from_slice(&(extra.len() as u32).to_be_bytes());
        records.extend_from_slice(&extra);
        for bytes in encoded {
            data.extend_from_slice(&bytes);
        }
    }
    let mut out = (doc.layers.len() as i16).to_be_bytes().to_vec();
    out.extend_from_slice(&records);
    out.extend_from_slice(&data);
    out
}

pub fn build(doc: &TestDoc) -> Vec<u8> {
    let wide = big(doc.version);
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&doc.version.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&doc.channels.to_be_bytes());
    out.extend_from_slice(&doc.height.to_be_bytes());
    out.extend_from_slice(&doc.width.to_be_bytes());
    out.extend_from_slice(&doc.depth.to_be_bytes());
    out.extend_from_slice(&doc.mode.to_be_bytes());

    // Colour mode data: none.
    out.extend_from_slice(&0_u32.to_be_bytes());

    // Image resources.
    let mut resources = Vec::new();
    if let Some(ppi) = doc.resolution_ppi {
        let fixed = (ppi * 65_536.0) as u32;
        let mut body = Vec::new();
        body.extend_from_slice(&fixed.to_be_bytes());
        body.extend_from_slice(&1_u16.to_be_bytes());
        body.extend_from_slice(&1_u16.to_be_bytes());
        body.extend_from_slice(&fixed.to_be_bytes());
        body.extend_from_slice(&1_u16.to_be_bytes());
        body.extend_from_slice(&1_u16.to_be_bytes());
        resources.extend_from_slice(b"8BIM");
        resources.extend_from_slice(&1005_u16.to_be_bytes());
        resources.extend_from_slice(&[0, 0]); // empty name, padded
        resources.extend_from_slice(&(body.len() as u32).to_be_bytes());
        resources.extend_from_slice(&body);
    }
    if let Some(icc) = &doc.icc {
        resources.extend_from_slice(b"8BIM");
        resources.extend_from_slice(&1039_u16.to_be_bytes());
        resources.extend_from_slice(&[0, 0]);
        resources.extend_from_slice(&(icc.len() as u32).to_be_bytes());
        resources.extend_from_slice(icc);
        if icc.len() % 2 == 1 {
            resources.push(0);
        }
    }
    out.extend_from_slice(&(resources.len() as u32).to_be_bytes());
    out.extend_from_slice(&resources);

    // Layer and mask information.
    let mut section = Vec::new();
    let layer_info = layer_records_and_data(doc);
    match doc.layers_in_block {
        None => {
            let mut padded = layer_info;
            if padded.len() % 2 == 1 {
                padded.push(0);
            }
            put_len(&mut section, padded.len() as u64, wide);
            section.extend_from_slice(&padded);
            section.extend_from_slice(&0_u32.to_be_bytes()); // global layer mask info
        }
        Some(key) => {
            put_len(&mut section, 0, wide);
            section.extend_from_slice(&0_u32.to_be_bytes());
            let mut padded = layer_info;
            while !padded.len().is_multiple_of(4) {
                padded.push(0);
            }
            block(&mut section, &key, &padded, wide && is_long(&key), 4);
        }
    }
    put_len(&mut out, section.len() as u64, wide);
    out.extend_from_slice(&section);

    // Merged image: raw.
    out.extend_from_slice(&0_u16.to_be_bytes());
    out.extend_from_slice(&doc.merged);
    out
}
