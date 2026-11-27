//! Clean-room Siemens NX (.prt) container reader and Parasolid XT stream extractor.
//!
//! Supports:
//! - Modern NX `SPLMSSTR` containers (zlib-compressed and raw binary XT records).
//! - Legacy NX `OLE2` / Microsoft Compound File Binary (MS-CFB) containers.
//! - Structured `0xEDF1D1C0` Parasolid chunk framing + safe fallback scanning inside `UG_PART`.
//! - Multi-stream deduplication and `BrepModel` merging, plus optional preview thumbnail extraction.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    extract_brep, BrepModel, Coedge, CurveData, CurveKind, Edge, Face, Loop, NurbsCurve,
    SurfaceData, SurfaceKind,
};
use crate::xb::{parse_xb, ParseResult, XtError, XtHeader};

pub const SPLMSSTR_MAGIC: &[u8; 8] = b"SPLMSSTR";
pub const OLE2_MAGIC: &[u8; 8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const PS_RECORD_MAGIC: &[u8; 4] = b"\xC0\xD1\xF1\xED";
const FREESECT: u32 = 0xFFFFFFFC;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrtContainerKind {
    Splmsstr,
    Ole2,
}

impl PrtContainerKind {
    pub fn format_kind(self) -> &'static str {
        match self {
            PrtContainerKind::Splmsstr => "nx-prt-splmsstr",
            PrtContainerKind::Ole2 => "nx-prt-ole2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PrtContainerKind::Splmsstr => "splmsstr",
            PrtContainerKind::Ole2 => "ole2",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrtStreamKind {
    Body,
    Partition,
    Delta,
}

#[derive(Clone, Debug)]
pub struct PrtStream {
    pub index: usize,
    pub kind: PrtStreamKind,
    pub offset: usize,
    pub compressed: bool,
    pub ascii_header: Vec<u8>,
    pub ps_bytes: Vec<u8>,
    pub standalone_xb: Vec<u8>,
    pub schema_name: String,
    pub body_count: usize,
    pub face_count: usize,
}

#[derive(Clone, Debug)]
pub struct PrtArchive {
    pub container: PrtContainerKind,
    pub streams: Vec<PrtStream>,
    pub preview: Option<Vec<u8>>,
    pub entry_names: Vec<String>,
}

pub fn is_prt_bytes(data: &[u8]) -> bool {
    data.starts_with(SPLMSSTR_MAGIC) || data.starts_with(OLE2_MAGIC)
}

fn read_u16_le(buf: &[u8], off: usize) -> Result<u16, XtError> {
    let slice = buf
        .get(off..off + 2)
        .ok_or_else(|| XtError::InvalidInput("Unexpected EOF reading u16".into()))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(buf: &[u8], off: usize) -> Result<u32, XtError> {
    let slice = buf
        .get(off..off + 4)
        .ok_or_else(|| XtError::InvalidInput("Unexpected EOF reading u32".into()))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(buf: &[u8], off: usize) -> Result<u64, XtError> {
    let slice = buf
        .get(off..off + 8)
        .ok_or_else(|| XtError::InvalidInput("Unexpected EOF reading u64".into()))?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

pub fn parse_splmsstr(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, XtError> {
    if data.len() < 35 || !data.starts_with(SPLMSSTR_MAGIC) {
        return Err(XtError::InvalidInput("Not a valid SPLMSSTR container".into()));
    }
    if &data[25..31] != b"HEADER" {
        return Err(XtError::InvalidInput(
            "Missing SPLMSSTR directory HEADER marker".into(),
        ));
    }

    let entry_count = read_u32_le(data, 31)? as usize;
    let mut pos = 35usize;
    let mut entries = BTreeMap::new();
    for _ in 0..entry_count {
        let path_len = read_u32_le(data, pos)? as usize;
        pos += 4;
        let path_bytes = data
            .get(pos..pos + path_len)
            .ok_or_else(|| XtError::InvalidInput("Truncated SPLMSSTR entry path".into()))?;
        let path = String::from_utf8_lossy(path_bytes).into_owned();
        pos += path_len;
        if path.ends_with('/') {
            if pos + 16 > data.len() {
                return Err(XtError::InvalidInput(
                    "Truncated SPLMSSTR directory GUID".into(),
                ));
            }
            pos += 16;
        } else {
            let off = read_u64_le(data, pos)? as usize;
            let size = read_u64_le(data, pos + 8)? as usize;
            pos += 16;
            let end = off
                .checked_add(size)
                .ok_or_else(|| XtError::InvalidInput("SPLMSSTR entry size overflow".into()))?;
            let slice = data
                .get(off..end)
                .ok_or_else(|| XtError::InvalidInput(format!("SPLMSSTR entry {path} out of bounds")))?;
            entries.insert(path, slice.to_vec());
        }
    }
    Ok(entries)
}

pub fn parse_ole2_cfb(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, XtError> {
    if data.len() < 512 || !data.starts_with(OLE2_MAGIC) {
        return Err(XtError::InvalidInput(
            "Not a valid OLE2 Compound File Binary container".into(),
        ));
    }
    let sec_shift = read_u16_le(data, 30)? as u32;
    let mini_shift = read_u16_le(data, 32)? as u32;
    if !(7..=16).contains(&sec_shift) || mini_shift > sec_shift {
        return Err(XtError::InvalidInput("Invalid OLE2 sector shifts".into()));
    }
    let sec_size = 1usize << sec_shift;
    let mini_size = 1usize << mini_shift;

    let dir_start = read_u32_le(data, 48)?;
    let mini_cutoff = read_u32_le(data, 56)? as usize;
    let minifat_start = read_u32_le(data, 60)?;
    let difat_start = read_u32_le(data, 68)?;
    let num_difat_sectors = read_u32_le(data, 72)? as usize;

    let mut fat_sectors = Vec::new();
    for i in 0..109 {
        let s = read_u32_le(data, 76 + i * 4)?;
        if s < FREESECT {
            fat_sectors.push(s);
        }
    }

    let ints_per_sec = sec_size / 4;
    let mut cur_difat = difat_start;
    let mut seen_difat = BTreeSet::new();
    while cur_difat < FREESECT && seen_difat.len() < num_difat_sectors.max(1) + 16 {
        if !seen_difat.insert(cur_difat) {
            break;
        }
        let off = (cur_difat as usize + 1) * sec_size;
        if off + sec_size > data.len() {
            break;
        }
        for i in 0..(ints_per_sec - 1) {
            let s = read_u32_le(data, off + i * 4)?;
            if s < FREESECT {
                fat_sectors.push(s);
            }
        }
        cur_difat = read_u32_le(data, off + (ints_per_sec - 1) * 4)?;
    }

    let mut fat = Vec::with_capacity(fat_sectors.len() * ints_per_sec);
    for &s in &fat_sectors {
        let off = (s as usize + 1) * sec_size;
        if off + sec_size > data.len() {
            break;
        }
        for i in 0..ints_per_sec {
            fat.push(read_u32_le(data, off + i * 4)?);
        }
    }

    let read_chain = |start: u32, max_len: Option<usize>| -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = start;
        let mut seen = BTreeSet::new();
        while s < FREESECT && (s as usize) < fat.len() && seen.insert(s) {
            let off = (s as usize + 1) * sec_size;
            if off + sec_size > data.len() {
                break;
            }
            out.extend_from_slice(&data[off..off + sec_size]);
            if let Some(limit) = max_len {
                if out.len() >= limit {
                    out.truncate(limit);
                    break;
                }
            }
            s = fat[s as usize];
        }
        if let Some(limit) = max_len {
            if out.len() > limit {
                out.truncate(limit);
            }
        }
        out
    };

    let dir_raw = read_chain(dir_start, None);
    if dir_raw.len() < 128 {
        return Err(XtError::InvalidInput("Empty or corrupt OLE2 directory".into()));
    }

    let root_start = read_u32_le(&dir_raw, 116)?;
    let root_size_raw = read_u64_le(&dir_raw, 120)?;
    let root_size = if sec_size == 512 {
        (root_size_raw & 0xFFFF_FFFF) as usize
    } else {
        root_size_raw as usize
    };
    let ministream = if root_start < FREESECT && root_size > 0 {
        read_chain(root_start, Some(root_size))
    } else {
        Vec::new()
    };

    let mut minifat = Vec::new();
    if minifat_start < FREESECT {
        let mf_raw = read_chain(minifat_start, None);
        for chunk in mf_raw.chunks_exact(4) {
            minifat.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
    }

    let read_mini = |start: u32, size: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(size);
        let mut s = start;
        let mut seen = BTreeSet::new();
        while s < FREESECT && out.len() < size && (s as usize) < minifat.len() && seen.insert(s) {
            let off = s as usize * mini_size;
            let end = (off + mini_size).min(ministream.len());
            if off < end {
                out.extend_from_slice(&ministream[off..end]);
            }
            if end < off + mini_size {
                break;
            }
            s = minifat[s as usize];
        }
        out.truncate(size);
        out
    };

    let mut streams = BTreeMap::new();
    for ent in dir_raw.chunks_exact(128) {
        let name_len = read_u16_le(ent, 64)? as usize;
        let obj_type = ent[66];
        if obj_type != 2 || name_len < 2 || name_len > 64 {
            continue;
        }
        let mut code_units = Vec::with_capacity((name_len - 2) / 2);
        for pair in ent[..name_len - 2].chunks_exact(2) {
            code_units.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
        let name = String::from_utf16_lossy(&code_units);
        let start = read_u32_le(ent, 116)?;
        let raw_sz = read_u64_le(ent, 120)?;
        let size = if sec_size == 512 {
            (raw_sz & 0xFFFF_FFFF) as usize
        } else {
            raw_sz as usize
        };
        let blob = if size == 0 {
            Vec::new()
        } else if size < mini_cutoff {
            read_mini(start, size)
        } else {
            read_chain(start, Some(size))
        };
        streams.insert(name, blob);
    }

    Ok(streams)
}

fn find_subslice_bounded(haystack: &[u8], needle: &[u8], limit: usize) -> Option<usize> {
    let end = haystack.len().min(limit);
    let sub = &haystack[..end];
    if needle.is_empty() || needle.len() > sub.len() {
        return None;
    }
    sub.windows(needle.len()).position(|w| w == needle)
}

fn is_zlib_header(buf: &[u8], pos: usize) -> bool {
    pos + 1 < buf.len()
        && buf[pos] == 0x78
        && matches!(buf[pos + 1], 0x01 | 0x5E | 0x9C | 0xDA)
}

fn make_standalone_xb(ascii_hdr: &[u8], ps_bytes: &[u8]) -> Vec<u8> {
    if ascii_hdr.starts_with(b"**ABC") {
        let mut end = ascii_hdr.len();
        while end > 0 && matches!(ascii_hdr[end - 1], b'+' | b' ' | b'\t' | b'\r' | b'\n') {
            end -= 1;
        }
        if end > 0 {
            let mut out = Vec::with_capacity(end + 1 + ps_bytes.len());
            out.extend_from_slice(&ascii_hdr[..end]);
            out.push(b'\n');
            out.extend_from_slice(ps_bytes);
            return out;
        }
    }
    ps_bytes.to_vec()
}

fn classify_stream(
    index: usize,
    offset: usize,
    compressed: bool,
    ascii_header: Vec<u8>,
    ps_bytes: Vec<u8>,
) -> Option<PrtStream> {
    let standalone = make_standalone_xb(&ascii_header, &ps_bytes);
    if let Ok(parsed) = parse_xb(&standalone) {
        let body_count = parsed.nodes.iter().filter(|n| n.type_id == 12).count();
        let face_count = parsed.nodes.iter().filter(|n| n.type_id == 14).count();
        let kind = if parsed.nodes.first().is_some_and(|n| n.name == "WORLD") {
            PrtStreamKind::Partition
        } else {
            PrtStreamKind::Body
        };
        return Some(PrtStream {
            index,
            kind,
            offset,
            compressed,
            ascii_header,
            ps_bytes,
            standalone_xb: standalone,
            schema_name: parsed.header.schema_name,
            body_count,
            face_count,
        });
    }

    // Check delta transmit marker (`PS\x00\x00` followed by schema header with `(deltas)` or PMARK records).
    let ps_head_end = ps_bytes.len().min(256);
    if find_subslice_bounded(&ps_bytes[..ps_head_end], b"(deltas)", ps_head_end).is_some()
        || find_subslice_bounded(&ascii_header, b"(deltas)", ascii_header.len()).is_some()
        || ps_bytes.get(4..8).is_some_and(|_| true)
    {
        return Some(PrtStream {
            index,
            kind: PrtStreamKind::Delta,
            offset,
            compressed,
            ascii_header,
            ps_bytes,
            standalone_xb: standalone,
            schema_name: String::new(),
            body_count: 0,
            face_count: 0,
        });
    }
    None
}

fn scan_ug_part_structured(ug: &[u8]) -> Vec<PrtStream> {
    let mut streams = Vec::new();
    let n = ug.len();
    let mut pos = 0usize;

    while pos + 12 <= n {
        let Some(rel) = ug[pos..].windows(4).position(|w| w == PS_RECORD_MAGIC) else {
            break;
        };
        let off = pos + rel;
        if off + 12 > n {
            break;
        }
        let flags = u32::from_le_bytes([ug[off + 4], ug[off + 5], ug[off + 6], ug[off + 7]]);
        let payload_len =
            u32::from_le_bytes([ug[off + 8], ug[off + 9], ug[off + 10], ug[off + 11]]) as usize;
        if flags == 0xFFFF_FFFF || flags == 0xEDF1_D1C0 || payload_len == 0 || payload_len > n {
            pos = off + 4;
            continue;
        }

        let is_zlib = (flags & 0x8000_0000) != 0;
        let has_ext = (flags & 0x4000_0000) != 0;
        let ascii_len = (flags & 0x3FFF_FFFF) as usize;
        if ascii_len > 65536 {
            pos = off + 4;
            continue;
        }

        let mut hdr_start: Option<usize> = None;
        if has_ext && off + 16 <= n {
            let ext_len =
                u32::from_le_bytes([ug[off + 12], ug[off + 13], ug[off + 14], ug[off + 15]]) as usize;
            if (16..=4096).contains(&ext_len) {
                for cand in [off + ext_len, off + ext_len + 4] {
                    let pstart = cand.saturating_add(ascii_len);
                    if pstart.saturating_add(payload_len) > n {
                        continue;
                    }
                    if ascii_len > 0 {
                        if ug.get(cand..cand + 5) == Some(b"**ABC".as_slice()) {
                            hdr_start = Some(cand);
                            break;
                        }
                    } else if is_zlib {
                        if is_zlib_header(ug, pstart) {
                            hdr_start = Some(cand);
                            break;
                        }
                    } else if ug.get(pstart..pstart + 4) == Some(b"PS\x00\x00".as_slice())
                        || ug.get(pstart..pstart + 5) == Some(b"+PS\x00\x00".as_slice())
                    {
                        hdr_start = Some(cand);
                        break;
                    }
                }
            }
        } else {
            let cand = off + 12;
            let pstart = cand.saturating_add(ascii_len);
            if pstart.saturating_add(payload_len) <= n {
                if ascii_len > 0 && ug.get(cand..cand + 5) == Some(b"**ABC".as_slice()) {
                    hdr_start = Some(cand);
                } else if ascii_len == 0 {
                    if is_zlib && is_zlib_header(ug, pstart) {
                        hdr_start = Some(cand);
                    } else if !is_zlib
                        && (ug.get(pstart..pstart + 4) == Some(b"PS\x00\x00".as_slice())
                            || ug.get(pstart..pstart + 5) == Some(b"+PS\x00\x00".as_slice()))
                    {
                        hdr_start = Some(cand);
                    }
                }
            }
        }

        let Some(hs) = hdr_start else {
            pos = off + 4;
            continue;
        };

        let payload_start = hs + ascii_len;
        let mut ascii_hdr = if ascii_len > 0 {
            ug[hs..payload_start].to_vec()
        } else {
            Vec::new()
        };
        let raw_slice = &ug[payload_start..payload_start + payload_len];
        let decompressed = if is_zlib {
            match miniz_oxide::inflate::decompress_to_vec_zlib(raw_slice) {
                Ok(v) => v,
                Err(_) => {
                    pos = off + 4;
                    continue;
                }
            }
        } else {
            raw_slice.to_vec()
        };

        let Some(ps_pos) = find_subslice_bounded(&decompressed, b"PS\x00\x00", 4096) else {
            pos = off + 4;
            continue;
        };
        if ascii_hdr.is_empty() && ps_pos > 0 {
            ascii_hdr = decompressed[..ps_pos].to_vec();
        }
        let ps_bytes = decompressed[ps_pos..].to_vec();
        if let Some(st) = classify_stream(streams.len(), off, is_zlib, ascii_hdr, ps_bytes) {
            streams.push(st);
            pos = payload_start + payload_len;
        } else {
            pos = off + 4;
        }
    }

    streams
}

fn inflate_zlib_prefix(input: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut state = miniz_oxide::inflate::stream::InflateState::new_boxed(miniz_oxide::DataFormat::Zlib);
    let mut out = Vec::new();
    let mut in_pos = 0usize;
    let mut chunk = [0u8; 4096];

    loop {
        let res = miniz_oxide::inflate::stream::inflate(
            &mut state,
            &input[in_pos..],
            &mut chunk,
            miniz_oxide::MZFlush::None,
        );
        in_pos += res.bytes_consumed;
        if res.bytes_written > 0 {
            out.extend_from_slice(&chunk[..res.bytes_written]);
        }
        match res.status {
            Ok(miniz_oxide::MZStatus::StreamEnd) => {
                return (out.len() >= 16 && in_pos >= 8).then_some((out, in_pos));
            }
            Ok(miniz_oxide::MZStatus::Ok) | Ok(miniz_oxide::MZStatus::NeedDict) => {
                if res.bytes_consumed == 0 && res.bytes_written == 0 {
                    return None;
                }
                if out.len() > 64 * 1024 * 1024 {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

fn scan_ug_part_fallback(ug: &[u8]) -> Vec<PrtStream> {
    let mut streams = Vec::new();
    let n = ug.len();
    let mut i = 0usize;

    while i + 2 < n {
        if is_zlib_header(ug, i) {
            if let Some((out, consumed)) = inflate_zlib_prefix(&ug[i..]) {
                if let Some(ps_pos) = find_subslice_bounded(&out, b"PS\x00\x00", 4096) {
                    let ascii_hdr = if ps_pos > 0 { out[..ps_pos].to_vec() } else { Vec::new() };
                    let ps_bytes = out[ps_pos..].to_vec();
                    if let Some(st) = classify_stream(streams.len(), i, true, ascii_hdr, ps_bytes) {
                        streams.push(st);
                        i += consumed;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    if !streams.is_empty() {
        return streams;
    }

    // Raw uncompressed PS\x00\x00 fallback
    let mut ps_offsets = Vec::new();
    let mut search_pos = 0usize;
    while search_pos + 4 <= n {
        if let Some(rel) = ug[search_pos..].windows(4).position(|w| w == b"PS\x00\x00") {
            let p = search_pos + rel;
            ps_offsets.push(p);
            search_pos = p + 4;
        } else {
            break;
        }
    }

    for idx in 0..ps_offsets.len() {
        let p_off = ps_offsets[idx];
        let end_off = ps_offsets.get(idx + 1).copied().unwrap_or(n);
        let hdr_start = p_off.saturating_sub(1024);
        let hdr_slice = &ug[hdr_start..p_off];
        let ascii_hdr = if let Some(abc) = hdr_slice.windows(5).rposition(|w| w == b"**ABC") {
            hdr_slice[abc..].to_vec()
        } else {
            Vec::new()
        };
        let ps_bytes = ug[p_off..end_off].to_vec();
        if let Some(st) = classify_stream(streams.len(), p_off, false, ascii_hdr, ps_bytes) {
            streams.push(st);
        }
    }
    streams
}

pub fn extract_prt(data: &[u8]) -> Result<PrtArchive, XtError> {
    let (container, entries, ug_part, preview) = if data.starts_with(SPLMSSTR_MAGIC) {
        let map = parse_splmsstr(data)?;
        let ug = map
            .get("/Root/UG_PART/UG_PART")
            .or_else(|| {
                map.iter()
                    .find(|(k, _)| k.to_ascii_uppercase().ends_with("/UG_PART"))
                    .map(|(_, v)| v)
            })
            .cloned();
        let prev = map.get("/Root/images/preview").cloned();
        (PrtContainerKind::Splmsstr, map, ug, prev)
    } else if data.starts_with(OLE2_MAGIC) {
        let map = parse_ole2_cfb(data)?;
        let ug = map
            .get("UG_PART")
            .or_else(|| {
                map.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("UG_PART"))
                    .map(|(_, v)| v)
            })
            .cloned();
        let prev = map.get("preview").cloned();
        (PrtContainerKind::Ole2, map, ug, prev)
    } else {
        return Err(XtError::InvalidInput(
            "Input is neither an NX SPLMSSTR nor an OLE2/CFB .prt file".into(),
        ));
    };

    let ug_bytes = ug_part.ok_or_else(|| {
        XtError::InvalidInput(format!(
            "NX .prt ({}) container has no UG_PART stream",
            container.label()
        ))
    })?;

    let mut streams = scan_ug_part_structured(&ug_bytes);
    if streams.is_empty() {
        streams = scan_ug_part_fallback(&ug_bytes);
    }

    Ok(PrtArchive {
        container,
        streams,
        preview,
        entry_names: entries.into_keys().collect(),
    })
}

pub fn parse_prt_primary(data: &[u8]) -> Result<ParseResult, XtError> {
    let archive = extract_prt(data)?;
    let chosen = archive
        .streams
        .iter()
        .filter(|s| s.kind != PrtStreamKind::Delta)
        .max_by_key(|s| (s.face_count, s.ps_bytes.len()))
        .ok_or_else(|| {
            XtError::Unsupported(format!(
                "NX .prt ({}) file contains no embedded Parasolid B-rep bodies (found {} stream(s); legacy internal storage or assembly-only part)",
                archive.container.label(),
                archive.streams.len()
            ))
        })?;
    let mut parsed = parse_xb(&chosen.standalone_xb)?;
    parsed.header.format_kind = archive.container.format_kind().to_string();
    Ok(parsed)
}

fn bump(id: i64, off: i64) -> i64 {
    if id > 0 {
        id + off
    } else {
        id
    }
}

fn bump_opt(id: Option<i64>, off: i64) -> Option<i64> {
    id.map(|v| bump(v, off))
}

fn max_brep_id(model: &BrepModel) -> i64 {
    let mut hi = 0i64;
    if let Some(&k) = model.vertices.keys().next_back() {
        hi = hi.max(k);
    }
    if let Some(&k) = model.edges.keys().next_back() {
        hi = hi.max(k);
    }
    if let Some(&k) = model.faces.keys().next_back() {
        hi = hi.max(k);
    }
    if let Some(&k) = model.surfaces.keys().next_back() {
        hi = hi.max(k);
    }
    if let Some(&k) = model.curves.keys().next_back() {
        hi = hi.max(k);
    }
    for b in &model.bodies {
        hi = hi.max(b.index);
        for &s in &b.shell_indices {
            hi = hi.max(s);
        }
    }
    for f in model.faces.values() {
        for lp in &f.loops {
            hi = hi.max(lp.index);
            for ce in &lp.coedges {
                hi = hi.max(ce.fin_index);
            }
        }
    }
    for a in &model.assemblies {
        hi = hi.max(a.index);
        for &ii in &a.instance_indices {
            hi = hi.max(ii);
        }
    }
    for ins in &model.instances {
        hi = hi.max(ins.index);
    }
    for at in &model.attributes {
        hi = hi.max(at.index);
    }
    hi
}

fn remap_curve(c: &CurveData, off: i64) -> CurveData {
    let sp_surface = c.sp_surface.as_ref().map(|s| remap_surface(s, off));
    let kind = match &c.kind {
        CurveKind::Nurbs(nc) => {
            let mut cloned: NurbsCurve = (**nc).clone();
            cloned.host_surface_index = bump_opt(cloned.host_surface_index, off);
            CurveKind::Nurbs(Box::new(cloned))
        }
        other => other.clone(),
    };
    CurveData {
        sense: c.sense,
        kind,
        sp_surface,
    }
}

fn remap_surface(s: &SurfaceData, off: i64) -> SurfaceData {
    let kind = match &s.kind {
        SurfaceKind::Swept {
            direction,
            scale,
            profile_index,
            profile,
        } => SurfaceKind::Swept {
            direction: *direction,
            scale: *scale,
            profile_index: bump_opt(*profile_index, off),
            profile: profile.as_ref().map(|c| Box::new(remap_curve(c, off))),
        },
        SurfaceKind::Spun {
            base,
            axis,
            x_axis,
            profile_index,
            profile,
        } => SurfaceKind::Spun {
            base: *base,
            axis: *axis,
            x_axis: *x_axis,
            profile_index: bump_opt(*profile_index, off),
            profile: profile.as_ref().map(|c| Box::new(remap_curve(c, off))),
        },
        SurfaceKind::Offset {
            offset,
            base_surface_index,
            base_surface,
        } => SurfaceKind::Offset {
            offset: *offset,
            base_surface_index: bump_opt(*base_surface_index, off),
            base_surface: base_surface.as_ref().map(|bs| Box::new(remap_surface(bs, off))),
        },
        other => other.clone(),
    };
    SurfaceData {
        sense: s.sense,
        kind,
    }
}

pub fn merge_brep_models(models: Vec<BrepModel>, header: Option<XtHeader>) -> Result<BrepModel, XtError> {
    if models.is_empty() {
        return Err(XtError::Unsupported(
            "No B-rep models available to merge".into(),
        ));
    }
    let mut iter = models.into_iter();
    let mut first = iter.next().unwrap();
    if iter.len() == 0 {
        if let Some(h) = header {
            first.header = h;
        }
        return Ok(first);
    }

    let final_header = header.unwrap_or_else(|| first.header.clone());
    let mut merged = BrepModel {
        header: final_header,
        bodies: Vec::new(),
        vertices: BTreeMap::new(),
        edges: BTreeMap::new(),
        faces: BTreeMap::new(),
        surfaces: BTreeMap::new(),
        curves: BTreeMap::new(),
        assemblies: Vec::new(),
        instances: Vec::new(),
        attributes: Vec::new(),
        warnings: Vec::new(),
    };

    let all_models = std::iter::once(first).chain(iter);
    let mut next_offset = 0i64;

    for (idx, src) in all_models.enumerate() {
        let off = next_offset;

        for (vid, mut v) in src.vertices {
            let nv = bump(vid, off);
            v.index = nv;
            merged.vertices.insert(nv, v);
        }
        for (sid, s) in src.surfaces {
            merged.surfaces.insert(bump(sid, off), remap_surface(&s, off));
        }
        for (cid, c) in src.curves {
            merged.curves.insert(bump(cid, off), remap_curve(&c, off));
        }
        for (eid, e) in src.edges {
            let ne = bump(eid, off);
            merged.edges.insert(
                ne,
                Edge {
                    index: ne,
                    node_id: e.node_id,
                    sense: e.sense,
                    tolerance: e.tolerance,
                    curve_index: bump_opt(e.curve_index, off),
                    curve: remap_curve(&e.curve, off),
                },
            );
        }
        for (fid, f) in src.faces {
            let nf = bump(fid, off);
            let loops = f
                .loops
                .into_iter()
                .map(|lp| Loop {
                    index: bump(lp.index, off),
                    coedges: lp
                        .coedges
                        .into_iter()
                        .map(|ce| Coedge {
                            edge_index: bump(ce.edge_index, off),
                            sense: ce.sense,
                            start_vertex: bump_opt(ce.start_vertex, off),
                            end_vertex: bump_opt(ce.end_vertex, off),
                            fin_index: bump(ce.fin_index, off),
                        })
                        .collect(),
                })
                .collect();
            merged.faces.insert(
                nf,
                Face {
                    index: nf,
                    node_id: f.node_id,
                    shell_index: bump_opt(f.shell_index, off),
                    sense: f.sense,
                    face_sense: f.face_sense,
                    surface_sense: f.surface_sense,
                    surface_index: bump_opt(f.surface_index, off),
                    surface: remap_surface(&f.surface, off),
                    loops,
                    tolerance: f.tolerance,
                    appearance: f.appearance,
                },
            );
        }
        for mut b in src.bodies {
            b.index = bump(b.index, off);
            b.shell_indices = b.shell_indices.into_iter().map(|s| bump(s, off)).collect();
            if b.name.is_none() {
                b.name = Some(format!("Part_{}", idx + 1));
            }
            merged.bodies.push(b);
        }
        for mut ins in src.instances {
            ins.index = bump(ins.index, off);
            ins.assembly_index = bump_opt(ins.assembly_index, off);
            ins.part_index = bump_opt(ins.part_index, off);
            merged.instances.push(ins);
        }
        for mut asm in src.assemblies {
            asm.index = bump(asm.index, off);
            asm.instance_indices = asm
                .instance_indices
                .into_iter()
                .map(|i| bump(i, off))
                .collect();
            merged.assemblies.push(asm);
        }
        for mut at in src.attributes {
            at.index = bump(at.index, off);
            at.owner_index = bump_opt(at.owner_index, off);
            merged.attributes.push(at);
        }
        merged.warnings.extend(src.warnings);

        next_offset = max_brep_id(&merged) + 1000;
    }

    Ok(merged)
}

pub fn extract_prt_brep(data: &[u8]) -> Result<BrepModel, XtError> {
    let archive = extract_prt(data)?;
    let mut seen_ps: Vec<&[u8]> = Vec::new();
    let mut models = Vec::new();

    for st in &archive.streams {
        if st.kind == PrtStreamKind::Delta {
            continue;
        }
        if seen_ps.iter().any(|&prev| prev == st.ps_bytes.as_slice()) {
            continue;
        }
        seen_ps.push(st.ps_bytes.as_slice());

        let mut parsed = parse_xb(&st.standalone_xb)?;
        parsed.header.format_kind = archive.container.format_kind().to_string();
        let m = extract_brep(&parsed);
        if !m.faces.is_empty() {
            models.push(m);
        }
    }

    if models.is_empty() {
        return Err(XtError::Unsupported(format!(
            "NX .prt ({}) file contains no embedded Parasolid B-rep bodies (found {} stream(s); legacy internal storage or assembly-only part)",
            archive.container.label(),
            archive.streams.len()
        )));
    }

    let mut hdr = models[0].header.clone();
    hdr.format_kind = archive.container.format_kind().to_string();
    merge_brep_models(models, Some(hdr))
}

/// Build a clean-room synthetic uncompressed `SPLMSSTR` v6 `.prt` archive from `.x_b` streams.
pub fn build_synthetic_splmsstr_prt(xb_streams: &[&[u8]], preview: Option<&[u8]>) -> Vec<u8> {
    let mut ug_buf = Vec::new();
    for v in [2u32, 1, 1, 1, 1, 32] {
        ug_buf.extend_from_slice(&v.to_le_bytes());
    }
    ug_buf.resize(ug_buf.len() + 16, 0);

    for &xb in xb_streams {
        let ps_pos = xb.windows(4).position(|w| w == b"PS\x00\x00").unwrap_or(0);
        let (ascii_hdr, ps_bytes) = (&xb[..ps_pos], &xb[ps_pos..]);
        let flags = 0x4000_0000u32 | ((ascii_hdr.len() as u32) & 0x3FFF_FFFF);
        let ext_len = 0x25u32;
        ug_buf.extend_from_slice(PS_RECORD_MAGIC);
        ug_buf.extend_from_slice(&flags.to_le_bytes());
        ug_buf.extend_from_slice(&(ps_bytes.len() as u32).to_le_bytes());
        ug_buf.extend_from_slice(&ext_len.to_le_bytes());
        ug_buf.resize(ug_buf.len() + (ext_len as usize - 16), 0);
        ug_buf.extend_from_slice(ascii_hdr);
        ug_buf.extend_from_slice(ps_bytes);
    }

    let mut entries: Vec<(&str, Option<&[u8]>)> = vec![
        ("/Root/", None),
        ("/Root/UG_PART/", None),
        ("/Root/UG_PART/UG_PART", Some(&ug_buf)),
    ];
    if let Some(prev) = preview {
        entries.push(("/Root/images/", None));
        entries.push(("/Root/images/preview", Some(prev)));
    }

    let dir_size: usize = entries.iter().map(|(p, _)| 4 + p.len() + 16).sum();
    let header_total = 35 + dir_size;

    let mut out = Vec::new();
    out.extend_from_slice(SPLMSSTR_MAGIC);
    out.push(6);
    out.resize(25, 0);
    out.extend_from_slice(b"HEADER");
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());

    let mut data_chunks = Vec::new();
    let mut cur_off = header_total as u64;
    let mut ug_off = 0u64;
    for (path, blob_opt) in entries {
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        if let Some(blob) = blob_opt {
            if path == "/Root/UG_PART/UG_PART" {
                ug_off = cur_off;
            }
            out.extend_from_slice(&cur_off.to_le_bytes());
            out.extend_from_slice(&(blob.len() as u64).to_le_bytes());
            data_chunks.extend_from_slice(blob);
            cur_off += blob.len() as u64;
        } else {
            out.resize(out.len() + 16, 0);
        }
    }
    let footer_off = header_total as u64 + data_chunks.len() as u64;
    out[9..17].copy_from_slice(&ug_off.to_le_bytes());
    out[17..25].copy_from_slice(&footer_off.to_le_bytes());
    out.extend_from_slice(&data_chunks);
    out
}

/// Build a clean-room minimal OLE2/CFB `.prt` archive from `.x_b` streams.
pub fn build_synthetic_ole2_prt(xb_streams: &[&[u8]]) -> Vec<u8> {
    let mut ug_buf = Vec::from(&b"..UGII  "[..]);
    ug_buf.resize(ug_buf.len() + 64, 0);
    for &xb in xb_streams {
        let ps_pos = xb.windows(4).position(|w| w == b"PS\x00\x00").unwrap_or(0);
        let (ascii_hdr, ps_bytes) = (&xb[..ps_pos], &xb[ps_pos..]);
        let flags = (ascii_hdr.len() as u32) & 0x3FFF_FFFF;
        ug_buf.extend_from_slice(PS_RECORD_MAGIC);
        ug_buf.extend_from_slice(&flags.to_le_bytes());
        ug_buf.extend_from_slice(&(ps_bytes.len() as u32).to_le_bytes());
        ug_buf.extend_from_slice(ascii_hdr);
        ug_buf.extend_from_slice(ps_bytes);
    }

    let sec_size = 512usize;
    let ug_secs = ug_buf.len().div_ceil(sec_size).max(1);
    let mut fat = vec![0xFFFF_FFFFu32; 128];
    fat[0] = 0xFFFF_FFFD; // FATSECT
    fat[1] = 0xFFFF_FFFE; // Directory ENDOFCHAIN
    for s in 2..(2 + ug_secs) {
        fat[s] = if s + 1 < 2 + ug_secs {
            (s + 1) as u32
        } else {
            0xFFFF_FFFE
        };
    }

    let make_dir = |name: &str, obj_type: u8, start: u32, size: u32, child: u32| -> [u8; 128] {
        let mut buf = [0u8; 128];
        let units: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let raw_len = units.len() * 2;
        for (i, u) in units.iter().enumerate() {
            buf[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        buf[64..66].copy_from_slice(&(raw_len as u16).to_le_bytes());
        buf[66] = obj_type;
        buf[67] = 1;
        buf[68..72].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        buf[72..76].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        buf[76..80].copy_from_slice(&child.to_le_bytes());
        buf[116..120].copy_from_slice(&start.to_le_bytes());
        buf[120..124].copy_from_slice(&size.to_le_bytes());
        buf
    };

    let mut dir_buf = vec![0u8; sec_size];
    dir_buf[0..128].copy_from_slice(&make_dir("Root Entry", 5, 0xFFFF_FFFE, 0, 1));
    dir_buf[128..256].copy_from_slice(&make_dir("UG_PART", 2, 2, ug_buf.len() as u32, 0xFFFF_FFFF));

    let mut hdr = vec![0u8; sec_size];
    hdr[0..8].copy_from_slice(OLE2_MAGIC);
    hdr[24..26].copy_from_slice(&0x003Eu16.to_le_bytes());
    hdr[26..28].copy_from_slice(&0x0003u16.to_le_bytes());
    hdr[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
    hdr[30..32].copy_from_slice(&9u16.to_le_bytes());
    hdr[32..34].copy_from_slice(&6u16.to_le_bytes());
    hdr[44..48].copy_from_slice(&1u32.to_le_bytes()); // num FAT sectors
    hdr[48..52].copy_from_slice(&1u32.to_le_bytes()); // dir start
    hdr[56..60].copy_from_slice(&0u32.to_le_bytes()); // mini cutoff = 0 -> regular FAT
    hdr[60..64].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
    hdr[68..72].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
    for i in 0..109 {
        let v = if i == 0 { 0u32 } else { 0xFFFF_FFFFu32 };
        hdr[76 + i * 4..80 + i * 4].copy_from_slice(&v.to_le_bytes());
    }

    let mut out = Vec::with_capacity((3 + ug_secs) * sec_size);
    out.extend_from_slice(&hdr);
    for v in fat {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&dir_buf);
    out.extend_from_slice(&ug_buf);
    out.resize((3 + ug_secs) * sec_size, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dumps_step;
    use crate::model::BrepModel;
    use crate::step_writer::{StepOptions, StepSchema, StepUnits};
    use crate::xt_writer::write_xb_from_brep;

    fn sample_box_xb(dx: f64, dy: f64, dz: f64) -> Vec<u8> {
        let positions = [
            [0.0, 0.0, 0.0],
            [dx, 0.0, 0.0],
            [dx, dy, 0.0],
            [0.0, dy, 0.0],
            [0.0, 0.0, dz],
            [dx, 0.0, dz],
            [dx, dy, dz],
            [0.0, dy, dz],
        ];
        let tris = [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        let mesh = crate::mesh_brep::MeshBodyInput {
            name: "Box",
            positions_mm: &positions,
            triangles: &tris,
            appearance: None,
        };
        let model: BrepModel = crate::mesh_brep::meshes_to_brep(&[mesh], true).unwrap();
        write_xb_from_brep(&model).unwrap()
    }

    #[test]
    fn synthetic_splmsstr_and_ole2_roundtrip() {
        let b1 = sample_box_xb(10.0, 10.0, 10.0);
        let b2 = sample_box_xb(5.0, 5.0, 5.0);

        // Multi-stream SPLMSSTR with duplicate b1 + distinct b2
        let prt = build_synthetic_splmsstr_prt(&[&b1, &b1, &b2], Some(b"\xff\xd8\xff\xd9"));
        assert!(is_prt_bytes(&prt));

        let arch = extract_prt(&prt).unwrap();
        assert_eq!(arch.container, PrtContainerKind::Splmsstr);
        assert_eq!(arch.streams.len(), 3);
        assert_eq!(arch.preview.as_deref(), Some(&b"\xff\xd8\xff\xd9"[..]));

        let merged = extract_prt_brep(&prt).unwrap();
        assert_eq!(merged.bodies.len(), 2);
        assert_eq!(merged.faces.len(), 12);

        let step = dumps_step(
            &merged,
            &StepOptions {
                schema: StepSchema::Ap214,
                units: StepUnits::Mm,
                name: "prt_multi".to_string(),
                ..Default::default()
            },
        );
        assert!(step.contains("CLOSED_SHELL"));

        // OLE2 CFB container
        let ole = build_synthetic_ole2_prt(&[&b1]);
        assert!(is_prt_bytes(&ole));
        let ole_arch = extract_prt(&ole).unwrap();
        assert_eq!(ole_arch.container, PrtContainerKind::Ole2);
        let ole_brep = extract_prt_brep(&ole).unwrap();
        assert_eq!(ole_brep.bodies.len(), 1);
        assert_eq!(ole_brep.faces.len(), 6);
    }

    #[test]
    fn empty_prt_returns_clear_error() {
        let fake_ug = b"NO_PARASOLID";
        let entries = [
            ("/Root/", false),
            ("/Root/UG_PART/", false),
            ("/Root/UG_PART/UG_PART", true),
        ];
        let dir_size: usize = entries.iter().map(|(p, _)| 4 + p.len() + 16).sum();
        let ug_off = (35 + dir_size) as u64;
        let mut buf = Vec::new();
        buf.extend_from_slice(SPLMSSTR_MAGIC);
        buf.push(6);
        buf.extend_from_slice(&ug_off.to_le_bytes());
        buf.extend_from_slice(&(ug_off + fake_ug.len() as u64).to_le_bytes());
        buf.extend_from_slice(b"HEADER");
        buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (p, is_file) in entries {
            buf.extend_from_slice(&(p.len() as u32).to_le_bytes());
            buf.extend_from_slice(p.as_bytes());
            if is_file {
                buf.extend_from_slice(&ug_off.to_le_bytes());
                buf.extend_from_slice(&(fake_ug.len() as u64).to_le_bytes());
            } else {
                buf.resize(buf.len() + 16, 0);
            }
        }
        buf.extend_from_slice(fake_ug);
        let err = extract_prt_brep(&buf).unwrap_err();
        assert!(err.to_string().contains("no embedded Parasolid"));
    }
}
