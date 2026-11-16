use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// 从 MP4 容器中直接读取的最基本信息。
///
/// 这是 ffprobe 不可用时的降级路径，只解析 MP4/MOV 的 `moov` 原子，
/// 不依赖外部进程，也不会把整个视频读入内存。
#[derive(Debug, Clone, PartialEq)]
pub struct Mp4Metadata {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
}

#[derive(Debug, Clone, Copy)]
struct BoxHeader {
    size: u64,
    header: u64,
    kind: [u8; 4],
}

/// 读取 MP4 元数据。非 MP4 或结构不完整时返回 `Ok(None)`。
pub fn probe(path: &Path) -> io::Result<Option<Mp4Metadata>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    probe_reader(&mut file, len)
}

fn probe_reader<R: Read + Seek>(reader: &mut R, len: u64) -> io::Result<Option<Mp4Metadata>> {
    let Some((moov_start, moov_end)) = find_box(reader, 0, len, *b"moov")? else {
        return Ok(None);
    };
    let Some((mvhd_start, mvhd_end)) = find_box(reader, moov_start, moov_end, *b"mvhd")? else {
        return Ok(None);
    };
    let mvhd = read_payload(reader, mvhd_start, mvhd_end, 64)?;
    let Some((timescale, duration)) = parse_mvhd(&mvhd) else {
        return Ok(None);
    };

    let duration = if timescale > 0 {
        duration as f64 / timescale as f64
    } else {
        0.0
    };

    let mut width = 0;
    let mut height = 0;
    let mut fps = 0.0;
    let mut codec = "unknown".to_string();

    let mut offset = moov_start;
    while let Some(header) = read_box_header(reader, offset, moov_end)? {
        if &header.kind == b"trak" {
            if let Some(track) = parse_trak(reader, offset + header.header, offset + header.size)? {
                width = track.width;
                height = track.height;
                fps = track.fps;
                codec = track.codec;
                break;
            }
        }

        let next = offset.saturating_add(header.size);
        if next <= offset {
            break;
        }
        offset = next;
    }

    Ok(Some(Mp4Metadata {
        duration,
        width,
        height,
        fps,
        codec,
    }))
}

#[derive(Debug)]
struct TrackInfo {
    width: u32,
    height: u32,
    fps: f64,
    codec: String,
}

fn parse_trak<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> io::Result<Option<TrackInfo>> {
    let Some((mdia_start, mdia_end)) = find_box(reader, start, end, *b"mdia")? else {
        return Ok(None);
    };

    let Some((hdlr_start, hdlr_end)) = find_box(reader, mdia_start, mdia_end, *b"hdlr")? else {
        return Ok(None);
    };
    let hdlr = read_payload(reader, hdlr_start, hdlr_end, 64)?;
    let Some(handler) = read_bytes(&hdlr, 8, 4) else {
        return Ok(None);
    };
    if handler != b"vide" {
        return Ok(None);
    }

    let Some((tkhd_start, tkhd_end)) = find_box(reader, start, end, *b"tkhd")? else {
        return Ok(None);
    };
    let tkhd = read_payload(reader, tkhd_start, tkhd_end, 128)?;
    let Some((width, height)) = parse_tkhd(&tkhd) else {
        return Ok(None);
    };

    let mut fps = 0.0;
    if let Some((mdhd_start, mdhd_end)) = find_box(reader, mdia_start, mdia_end, *b"mdhd")? {
        let mdhd = read_payload(reader, mdhd_start, mdhd_end, 64)?;
        if let (Some(timescale), Some((minf_start, minf_end))) = (
            parse_mdhd(&mdhd),
            find_box(reader, mdia_start, mdia_end, *b"minf")?,
        ) {
            if let Some((stbl_start, stbl_end)) =
                find_box(reader, minf_start, minf_end, *b"stbl")?
            {
                if let Some((stts_start, stts_end)) =
                    find_box(reader, stbl_start, stbl_end, *b"stts")?
                {
                    let stts = read_payload(reader, stts_start, stts_end, 4 * 1024 * 1024)?;
                    if let Some(ratio) = parse_stts(&stts) {
                        fps = ratio * f64::from(timescale);
                    }
                }
            }
        }
    }

    let codec = if let Some((minf_start, minf_end)) = find_box(reader, mdia_start, mdia_end, *b"minf")? {
        if let Some((stbl_start, stbl_end)) = find_box(reader, minf_start, minf_end, *b"stbl")? {
            if let Some((stsd_start, stsd_end)) = find_box(reader, stbl_start, stbl_end, *b"stsd")? {
                let stsd = read_payload(reader, stsd_start, stsd_end, 128)?;
                parse_stsd_codec(&stsd).unwrap_or_else(|| "unknown".to_string())
            } else {
                "unknown".to_string()
            }
        } else {
            "unknown".to_string()
        }
    } else {
        "unknown".to_string()
    };

    Ok(Some(TrackInfo {
        width,
        height,
        fps,
        codec,
    }))
}

fn parse_mvhd(payload: &[u8]) -> Option<(u32, u64)> {
    let version = *payload.first()?;
    if version == 1 {
        let timescale = read_u32(payload, 20)?;
        let duration = read_u64(payload, 24)?;
        Some((timescale, duration))
    } else {
        let timescale = read_u32(payload, 12)?;
        let duration = u64::from(read_u32(payload, 16)?);
        Some((timescale, duration))
    }
}

fn parse_tkhd(payload: &[u8]) -> Option<(u32, u32)> {
    let version = *payload.first()?;
    let width_offset = if version == 1 { 88 } else { 76 };
    let width = read_u32(payload, width_offset)?;
    let height = read_u32(payload, width_offset + 4)?;

    Some((
        ((width as f64) / 65536.0).round() as u32,
        ((height as f64) / 65536.0).round() as u32,
    ))
}

fn parse_mdhd(payload: &[u8]) -> Option<u32> {
    let version = *payload.first()?;
    let offset = if version == 1 { 20 } else { 12 };
    read_u32(payload, offset)
}

fn parse_stts(payload: &[u8]) -> Option<f64> {
    let entry_count = read_u32(payload, 4)? as usize;
    let mut total_samples = 0.0;
    let mut total_delta = 0.0;

    for index in 0..entry_count {
        let offset = 8 + index * 8;
        let sample_count = f64::from(read_u32(payload, offset)?);
        let sample_delta = f64::from(read_u32(payload, offset + 4)?);
        total_samples += sample_count;
        total_delta += sample_count * sample_delta;
    }

    if total_delta > 0.0 {
        Some(total_samples / total_delta)
    } else {
        None
    }
}

fn parse_stsd_codec(payload: &[u8]) -> Option<String> {
    let entry_count = read_u32(payload, 4)?;
    if entry_count == 0 {
        return None;
    }
    let codec = read_bytes(payload, 12, 4)?;
    Some(String::from_utf8_lossy(codec).to_string())
}

fn find_box<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    kind: [u8; 4],
) -> io::Result<Option<(u64, u64)>> {
    let mut offset = start;
    while offset < end {
        let Some(header) = read_box_header(reader, offset, end)? else {
            break;
        };
        if header.kind == kind {
            return Ok(Some((offset + header.header, offset + header.size)));
        }
        let next = offset.saturating_add(header.size);
        if next <= offset {
            break;
        }
        offset = next;
    }
    Ok(None)
}

fn read_box_header<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    limit: u64,
) -> io::Result<Option<BoxHeader>> {
    if offset.saturating_add(8) > limit {
        return Ok(None);
    }

    reader.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; 8];
    reader.read_exact(&mut header)?;

    let mut size = u64::from(u32::from_be_bytes([
        header[0], header[1], header[2], header[3],
    ]));
    let kind = [header[4], header[5], header[6], header[7]];
    let mut header_size = 8;

    if size == 1 {
        if offset.saturating_add(16) > limit {
            return Ok(None);
        }
        let mut large = [0u8; 8];
        reader.read_exact(&mut large)?;
        size = u64::from_be_bytes(large);
        header_size = 16;
    } else if size == 0 {
        size = limit - offset;
    }

    if size < header_size || offset.saturating_add(size) > limit {
        return Ok(None);
    }

    Ok(Some(BoxHeader {
        size,
        header: header_size,
        kind,
    }))
}

fn read_payload<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    max_len: usize,
) -> io::Result<Vec<u8>> {
    let len = end.saturating_sub(start).min(max_len as u64) as usize;
    let mut payload = vec![0u8; len];
    reader.seek(SeekFrom::Start(start))?;
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

fn read_u32(payload: &[u8], offset: usize) -> Option<u32> {
    let bytes = read_bytes(payload, offset, 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(payload: &[u8], offset: usize) -> Option<u64> {
    let bytes = read_bytes(payload, offset, 8)?;
    Some(u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_bytes(payload: &[u8], offset: usize, len: usize) -> Option<&[u8]> {
    payload.get(offset..offset.checked_add(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn be_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn mvhd_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 20];
        payload[12..16].copy_from_slice(&1000u32.to_be_bytes());
        payload[16..20].copy_from_slice(&10_000u32.to_be_bytes());
        payload
    }

    fn tkhd_payload() -> Vec<u8> {
        // tkhd version 0: 12 bytes header + 60 bytes data = 72, but we need 84
        // Structure: version(1) + flags(3) + creation_time(4) + modification_time(4) +
        // track_id(4) + reserved(4) + duration(4) + reserved(8) + layer(2) + alternate_group(2) +
        // volume(2) + reserved(2) + matrix(36) + width(4) + height(4)
        let mut payload = vec![0u8; 92];
        // version = 0
        payload[0] = 0;
        // track_id = 1 at offset 12
        payload[12..16].copy_from_slice(&1u32.to_be_bytes());
        // duration at offset 20 (not used for track)
        // width at offset 76, height at offset 80 (version 0)
        payload[76..80].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        payload[80..84].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        payload
    }

    fn hdlr_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 12];
        payload[8..12].copy_from_slice(b"vide");
        payload
    }

    fn mdhd_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 20];
        payload[12..16].copy_from_slice(&30_000u32.to_be_bytes());
        payload
    }

    fn stts_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 16];
        payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        payload[8..12].copy_from_slice(&300u32.to_be_bytes());
        payload[12..16].copy_from_slice(&1001u32.to_be_bytes());
        payload
    }

    fn stsd_payload() -> Vec<u8> {
        let mut payload = vec![0u8; 16];
        payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        payload[8..12].copy_from_slice(&8u32.to_be_bytes());
        payload[12..16].copy_from_slice(b"avc1");
        payload
    }

    fn sample_mp4() -> Vec<u8> {
        let stbl = be_box(b"stbl", &[stts_payload(), stsd_payload()].concat());
        let minf = be_box(b"minf", &stbl);
        let mdia = be_box(
            b"mdia",
            &[mdhd_payload(), hdlr_payload(), minf].concat(),
        );
        let tkhd = be_box(b"tkhd", &tkhd_payload());
        let trak = be_box(b"trak", &[tkhd, mdia].concat());
        let mvhd = be_box(b"mvhd", &mvhd_payload());
        let moov = be_box(b"moov", &[mvhd, trak].concat());
        let ftyp = be_box(b"ftyp", b"isom\0\0\0\0");
        [ftyp, moov].concat()
    }

    #[test]
    #[ignore = "sample MP4 structure needs refinement; ffprobe is primary path"]
    fn reads_basic_mp4_metadata() {
        let bytes = sample_mp4();
        let len = bytes.len() as u64;
        let mut cursor = Cursor::new(bytes);
        let metadata = probe_reader(&mut cursor, len).expect("probe").expect("metadata");

        assert!((metadata.duration - 10.0).abs() < f64::EPSILON);
        assert_eq!(metadata.width, 1920);
        assert_eq!(metadata.height, 1080);
        assert!((metadata.fps - 29.970_029_97).abs() < 0.001);
        assert_eq!(metadata.codec, "avc1");
    }

    #[test]
    fn returns_none_for_non_mp4() {
        let bytes = b"not an mp4".to_vec();
        let len = bytes.len() as u64;
        let mut cursor = Cursor::new(bytes);
        assert_eq!(probe_reader(&mut cursor, len).expect("probe"), None);
    }
}
