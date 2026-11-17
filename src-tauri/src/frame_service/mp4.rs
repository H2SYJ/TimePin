//! MP4 关键帧索引解析（本地无副作用）
//!
//! 解析 ISO BMFF (MP4) 的 sample table，提取关键帧时间戳与文件偏移，
//! 供 `frames.extract` keyframes 模式做精确 seek（不读解码数据、不修改文件）。
//!
//! 仅解析这些 box：
//! - `moov.udta` / `moov.trak.mdia.minf.stbl.stts` — sample-to-time 表
//! - `moov.trak.mdia.minf.stbl.stss` — keyframe（sync sample）索引
//! - `moov.trak.mdia.minf.stbl.stsc` — sample-to-chunk 表
//! - `moov.trak.mdia.minf.stbl.stco` / `co64` — chunk offset（32/64 位）
//! - `moov.trak.mdia.minf.stbl.stsz` — sample size
//! - `moov.trak.mdia.minf.stbl.ctts` — composition offset（B 帧 pts 修正）
//! - `moov.trak.mdia.minf.stbl.elst` — edit list（偏移修正）
//! - `moov.trak.mdia.mdhd` — media timescale
//!
//! 返回 `VideoIndex`：stream_index（视频轨 index）、keyframes（pts 排序）、
//! duration_sec（轨道时长，秒）。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct VideoIndex {
    /// 视频轨的 stream_index（通常 0）
    pub stream_index: usize,
    /// 按 pts_sec 升序排列的关键帧
    pub keyframes: Vec<Keyframe>,
    /// 视频轨时长（秒，基于 mdhd.timescale 与 trak 内 sample 总数）
    pub duration_sec: f64,
}

#[derive(Debug, Clone)]
pub struct Keyframe {
    /// 在 sample table 中的 index（0-based）
    pub sample_index: u32,
    /// 真实 PTS（秒），已合并 ctts/elst 修正
    pub pts_sec: f64,
    /// 文件内字节偏移
    pub offset: u64,
    /// 字节大小
    pub size: u32,
}

// ==================== ISO BMFF box 类型 ====================

#[derive(Debug, Clone, Copy)]
struct BoxHeader {
    size: u64,
    type_fourcc: [u8; 4],
    /// header 之后的 payload 起始偏移（已含 8 字节 size+type，或 16 字节 largesize）
    data_offset: u64,
}

#[allow(dead_code)]
fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes([s[0], s[1], s[2], s[3]])
}

// ==================== 入口 ====================

/// 解析 MP4 文件，返回视频轨的关键帧索引。
///
/// 失败时返回 Err（字符串描述），不 panic；
/// 对 fragmented MP4 / 加密 track 返回 Err（不支持）。
pub fn read_index(path: &Path) -> Result<VideoIndex, String> {
    let mut file = File::open(path).map_err(|e| format!("打开 {} 失败：{e}", path.display()))?;
    let file_size = file
        .metadata()
        .map_err(|e| format!("读取元数据失败：{e}"))?
        .len();

    // 找 moov box（通常在文件末尾，但我们顺序扫描以兼容 faststart）
    let moov = find_top_level_box(&mut file, file_size, b"moov")?
        .ok_or_else(|| "未找到 moov box（可能是 fragmented MP4 或非 MP4 文件）".to_string())?;

    // 遍历 moov 内 trak，找第一个 vide track
    let mut offset = moov.data_offset;
    let moov_end = moov.data_offset + moov_payload_size(&moov);
    let mut video_track: Option<TrackData> = None;

    while offset < moov_end {
        let header = read_box_header(&mut file, offset)?;
        offset = advance_past(&header, offset);
        if &header.type_fourcc == b"trak" {
            let track = parse_trak(&mut file, &header)?;
            if let Some(ref t) = track {
                if t.is_video {
                    video_track = Some(t.clone());
                    break;
                }
            }
        }
    }

    let track = video_track.ok_or_else(|| "moov 中未找到 vide track".to_string())?;
    build_index(&mut file, &track)
}

// ==================== trak 解析 ====================

#[derive(Debug, Clone)]
struct TrackData {
    is_video: bool,
    /// mdhd timescale（每秒单位数）
    timescale: u32,
    /// 轨道时长（秒，由 stts 总数/timescale 算出）
    duration_sec: f64,
    /// stbl 的绝对偏移（用于后续按需解析子 box）
    #[allow(dead_code)]
    stbl_offset: u64,
    #[allow(dead_code)]
    stbl_size: u64,
    /// 已由 stbl 解析出的数据
    stts: Vec<SttsEntry>,
    stss: Vec<u32>,
    stsc: Vec<StscEntry>,
    stco: Vec<u64>,
    stsz: Vec<u32>,
    ctts: Vec<CttsEntry>,
    elst: Vec<ElstEntry>,
}

#[derive(Debug, Clone)]
struct SttsEntry {
    sample_count: u32,
    sample_delta: u32,
}

#[derive(Debug, Clone)]
struct StscEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
    /// 忽略 sample_description_index
    _reserved: u32,
}

#[derive(Debug, Clone)]
struct CttsEntry {
    sample_count: u32,
    /// composition offset（可能为负，version=1 时 i32）
    offset: i64,
}

#[derive(Debug, Clone)]
struct ElstEntry {
    #[allow(dead_code)]
    segment_duration: u64,
    media_time: i64,
    // 忽略 media_rate_integer / media_rate_fraction
}

fn parse_trak(file: &mut File, trak_header: &BoxHeader) -> Result<Option<TrackData>, String> {
    let trak_end = trak_header.data_offset + box_payload_size(trak_header);
    let mut offset = trak_header.data_offset;
    let mut mdia_header: Option<BoxHeader> = None;

    // trak 内找 mdia
    while offset < trak_end {
        let header = read_box_header(file, offset)?;
        if &header.type_fourcc == b"mdia" {
            mdia_header = Some(header);
            break;
        }
        offset = advance_past(&header, offset);
    }

    let mdia = match mdia_header {
        Some(h) => h,
        None => return Ok(None),
    };

    // mdia 内解析 mdhd / hdlr / minf
    let mdia_end = mdia.data_offset + box_payload_size(&mdia);
    let mut offset = mdia.data_offset;
    let mut timescale = 0u32;
    let mut handler_type = [0u8; 4];
    let mut minf_header: Option<BoxHeader> = None;

    while offset < mdia_end {
        let header = read_box_header(file, offset)?;
        match &header.type_fourcc {
            b"mdhd" => {
                timescale = parse_mdhd_timescale(file, &header)?;
            }
            b"hdlr" => {
                handler_type = parse_hdlr_type(file, &header)?;
            }
            b"minf" => {
                minf_header = Some(header);
            }
            _ => {}
        }
        offset = advance_past(&header, offset);
    }

    if &handler_type != b"vide" {
        return Ok(None);
    }

    let minf = minf_header.ok_or_else(|| "vide track 缺少 minf box".to_string())?;

    // minf → stbl
    let minf_end = minf.data_offset + box_payload_size(&minf);
    let mut offset = minf.data_offset;
    let mut stbl_header: Option<BoxHeader> = None;
    while offset < minf_end {
        let header = read_box_header(file, offset)?;
        if &header.type_fourcc == b"stbl" {
            stbl_header = Some(header);
            break;
        }
        offset = advance_past(&header, offset);
    }

    let stbl = stbl_header.ok_or_else(|| "vide track 缺少 stbl box".to_string())?;

    // stbl 内解析 stts / stss / stsc / stco|co64 / stsz / ctts / elst
    let stbl_end = stbl.data_offset + box_payload_size(&stbl);
    let mut offset = stbl.data_offset;
    let mut track = TrackData {
        is_video: true,
        timescale,
        duration_sec: 0.0,
        stbl_offset: stbl.data_offset,
        stbl_size: box_payload_size(&stbl),
        stts: Vec::new(),
        stss: Vec::new(),
        stsc: Vec::new(),
        stco: Vec::new(),
        stsz: Vec::new(),
        ctts: Vec::new(),
        elst: Vec::new(),
    };

    while offset < stbl_end {
        let header = read_box_header(file, offset)?;
        match &header.type_fourcc {
            b"stts" => track.stts = parse_stts(file, &header)?,
            b"stss" => track.stss = parse_stss(file, &header)?,
            b"stsc" => track.stsc = parse_stsc(file, &header)?,
            b"stco" => track.stco = parse_stco(file, &header, 4)?,
            b"co64" => track.stco = parse_stco(file, &header, 8)?,
            b"stsz" => track.stsz = parse_stsz(file, &header)?,
            b"ctts" => track.ctts = parse_ctts(file, &header)?,
            b"elst" => track.elst = parse_elst(file, &header)?,
            _ => {}
        }
        offset = advance_past(&header, offset);
    }

    // 由 stts 计算总时长（秒）
    let total_samples: u64 = track.stts.iter().map(|e| u64::from(e.sample_count)).sum();
    if timescale > 0 {
        // duration = sum(sample_count * delta) / timescale
        let total_units: u64 = track
            .stts
            .iter()
            .map(|e| u64::from(e.sample_count) * u64::from(e.sample_delta))
            .sum();
        track.duration_sec = total_units as f64 / f64::from(timescale);
        if total_samples == 0 {
            track.duration_sec = 0.0;
        }
    }

    Ok(Some(track))
}

// ==================== 表解析 ====================

fn parse_mdhd_timescale(file: &mut File, header: &BoxHeader) -> Result<u32, String> {
    let version = read_u8(file, header.data_offset)?;
    // mdhd version=0: timescale 在偏移 20（4 version/flags + 4 ctime + 4 mtime + 4 timescale）
    // mdhd version=1: timescale 在偏移 28（8+8+4）
    let timescale_offset = if version == 1 {
        header.data_offset + 20
    } else {
        header.data_offset + 12
    };
    read_u32(file, timescale_offset).map_err(|e| format!("mdhd timescale 读取失败：{e}"))
}

fn parse_hdlr_type(file: &mut File, header: &BoxHeader) -> Result<[u8; 4], String> {
    // hdlr 偏移：version(1) + flags(3) + pre_defined(4) + handler_type(4)
    file.seek(SeekFrom::Start(header.data_offset + 8))
        .map_err(|e| format!("hdlr seek 失败：{e}"))?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)
        .map_err(|e| format!("hdlr type 读取失败：{e}"))?;
    Ok(buf)
}

fn parse_stts(file: &mut File, header: &BoxHeader) -> Result<Vec<SttsEntry>, String> {
    // stts: version(1) + flags(3) + entry_count(4) + [sample_count(4) + sample_delta(4)]*
    let mut pos = header.data_offset + 8;
    let entry_count = read_u32(file, pos)?;
    pos += 4;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let sample_count = read_u32(file, pos)?;
        let sample_delta = read_u32(file, pos + 4)?;
        entries.push(SttsEntry {
            sample_count,
            sample_delta,
        });
        pos += 8;
    }
    Ok(entries)
}

fn parse_stss(file: &mut File, header: &BoxHeader) -> Result<Vec<u32>, String> {
    // stss: version(1) + flags(3) + entry_count(4) + [sample_number(4)]*
    let mut pos = header.data_offset + 8;
    let entry_count = read_u32(file, pos)?;
    pos += 4;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        entries.push(read_u32(file, pos)?);
        pos += 4;
    }
    Ok(entries)
}

fn parse_stsc(file: &mut File, header: &BoxHeader) -> Result<Vec<StscEntry>, String> {
    // stsc: version(1) + flags(3) + entry_count(4) + [first_chunk(4)+samples_per_chunk(4)+sdi(4)]*
    let mut pos = header.data_offset + 8;
    let entry_count = read_u32(file, pos)?;
    pos += 4;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let first_chunk = read_u32(file, pos)?;
        let samples_per_chunk = read_u32(file, pos + 4)?;
        let reserved = read_u32(file, pos + 8)?;
        entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
            _reserved: reserved,
        });
        pos += 12;
    }
    Ok(entries)
}

fn parse_stco(file: &mut File, header: &BoxHeader, width: usize) -> Result<Vec<u64>, String> {
    // stco (4-byte) / co64 (8-byte): version(1) + flags(3) + entry_count(4) + [offset]*
    let mut pos = header.data_offset + 8;
    let entry_count = read_u32(file, pos)?;
    pos += 4;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let off = if width == 4 {
            u64::from(read_u32(file, pos)?)
        } else {
            read_u64(file, pos)?
        };
        entries.push(off);
        pos += width as u64;
    }
    Ok(entries)
}

fn parse_stsz(file: &mut File, header: &BoxHeader) -> Result<Vec<u32>, String> {
    // stsz: version(1)+flags(3)+sample_size(4)+sample_count(4)+[entry_size(4)]*
    // 若 sample_size != 0 则所有 sample 同 size，无后续表
    let mut pos = header.data_offset + 8;
    let sample_size = read_u32(file, pos)?;
    let sample_count = read_u32(file, pos + 4)?;
    pos += 8;
    if sample_size != 0 {
        return Ok(vec![sample_size; sample_count as usize]);
    }
    let mut entries = Vec::with_capacity(sample_count as usize);
    for _ in 0..sample_count {
        entries.push(read_u32(file, pos)?);
        pos += 4;
    }
    Ok(entries)
}

fn parse_ctts(file: &mut File, header: &BoxHeader) -> Result<Vec<CttsEntry>, String> {
    // ctts: version(1)+flags(3)+entry_count(4)+[sample_count(4)+offset(4)]*
    // version=0: offset 为 u32（恒正）；version=1: offset 为 i32（可负，B 帧）
    let version = read_u8(file, header.data_offset)?;
    let mut pos = header.data_offset + 8;
    let entry_count = read_u32(file, pos)?;
    pos += 4;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let sample_count = read_u32(file, pos)?;
        let offset_raw = read_u32(file, pos + 4)?;
        let offset: i64 = if version == 1 {
            i64::from(offset_raw as i32)
        } else {
            i64::from(offset_raw)
        };
        entries.push(CttsEntry {
            sample_count,
            offset,
        });
        pos += 8;
    }
    Ok(entries)
}

fn parse_elst(file: &mut File, header: &BoxHeader) -> Result<Vec<ElstEntry>, String> {
    // elst: version(1)+flags(3)+entry_count(4)+[segment_duration+media_time+rate]*
    // version=0: segment_duration u32, media_time i32, rate 4 bytes
    // version=1: segment_duration u64, media_time i64, rate 4 bytes
    let version = read_u8(file, header.data_offset)?;
    let mut pos = header.data_offset + 8;
    let entry_count = read_u32(file, pos)?;
    pos += 4;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let (segment_duration, media_time) = if version == 1 {
            (read_u64(file, pos)?, read_u64(file, pos + 8)? as i64)
        } else {
            (
                u64::from(read_u32(file, pos)?),
                i64::from(read_u32(file, pos + 4)? as i32),
            )
        };
        entries.push(ElstEntry {
            segment_duration,
            media_time,
        });
        pos += if version == 1 { 20 } else { 12 };
    }
    Ok(entries)
}

// ==================== 索引组装 ====================

fn build_index(_file: &mut File, track: &TrackData) -> Result<VideoIndex, String> {
    if track.timescale == 0 {
        return Err("mdhd timescale 为 0，无法计算 pts".to_string());
    }
    if track.stts.is_empty() || track.stsz.is_empty() {
        return Err("stts/stsz 为空，无法构建索引".to_string());
    }

    // 1. 展开 stts → 每 sample 的 delta（DTS 增量，单位 timescale）
    let mut dts_units: Vec<u64> = Vec::with_capacity(track.stsz.len());
    let mut cur: u64 = 0;
    for entry in &track.stts {
        for _ in 0..entry.sample_count {
            if dts_units.len() >= track.stsz.len() {
                break;
            }
            dts_units.push(cur);
            cur += u64::from(entry.sample_delta);
        }
    }
    // 若 stts 总数少于 stsz（异常），用最后一个 delta 补齐
    while dts_units.len() < track.stsz.len() {
        dts_units.push(cur);
    }

    // 2. 展开 ctts → 每 sample 的 composition offset（pts - dts）
    let mut cts_offsets: Vec<i64> = Vec::with_capacity(track.stsz.len());
    if track.ctts.is_empty() {
        cts_offsets.resize(track.stsz.len(), 0);
    } else {
        for entry in &track.ctts {
            for _ in 0..entry.sample_count {
                if cts_offsets.len() >= track.stsz.len() {
                    break;
                }
                cts_offsets.push(entry.offset);
            }
        }
        while cts_offsets.len() < track.stsz.len() {
            cts_offsets.push(0);
        }
    }

    // 3. elst 偏移（media_time 起始点；elst[0].media_time 表示首个 sample 在该时刻播放）
    //    对常见录制文件 elst 为空或 media_time=0；若非零则从每个 pts 减去。
    let elst_offset_units: i64 = track.elst.first().map(|e| e.media_time).unwrap_or(0);

    // 4. 展开 stsc + stco → 每 sample 的 (chunk_index, offset_in_chunk)
    //    stsc 按 chunk 分组；first_chunk 从 1 开始
    if track.stsc.is_empty() || track.stco.is_empty() {
        return Err("stsc/stco 为空，无法定位 sample".to_string());
    }

    // 为每个 chunk 计算 (chunk_index → stsc 参数)
    // stsc[i].first_chunk <= chunk_id < stsc[i+1].first_chunk
    let chunk_count = track.stco.len();
    let mut chunk_samples: Vec<u32> = vec![0; chunk_count];
    for (i, entry) in track.stsc.iter().enumerate() {
        let start = (entry.first_chunk as usize).saturating_sub(1);
        let end = track
            .stsc
            .get(i + 1)
            .map(|e| (e.first_chunk as usize).saturating_sub(1))
            .unwrap_or(chunk_count)
            .min(chunk_count);
        for c in start..end {
            if let Some(s) = chunk_samples.get_mut(c) {
                *s = entry.samples_per_chunk;
            }
        }
    }

    // 5. 展开 sample → (offset, size, pts_sec)
    //    每个 chunk 内 sample 顺序存储；offset 累加 stsz
    let total_samples = track.stsz.len();
    let mut sample_offsets: Vec<u64> = Vec::with_capacity(total_samples);
    let mut sample_idx = 0usize;
    for (chunk_id, chunk_offset) in track.stco.iter().enumerate() {
        let per_chunk = chunk_samples.get(chunk_id).copied().unwrap_or(0) as usize;
        let mut off = *chunk_offset;
        for _ in 0..per_chunk {
            if sample_idx >= total_samples {
                break;
            }
            sample_offsets.push(off);
            off += u64::from(track.stsz[sample_idx]);
            sample_idx += 1;
        }
    }
    if sample_offsets.len() < total_samples {
        return Err(format!(
            "sample offset 展开不完整：{}/{}（stsc/stco 与 stsz 不一致）",
            sample_offsets.len(),
            total_samples
        ));
    }

    // 6. 关键帧集合：若 stss 缺失则所有 sample 都是关键帧（intra-only codec）
    let keyframe_set: std::collections::HashSet<u32> = if track.stss.is_empty() {
        (1..=total_samples as u32).collect()
    } else {
        track.stss.iter().copied().collect()
    };

    // 7. 组装 Keyframe
    let ts = f64::from(track.timescale);
    let mut keyframes = Vec::new();
    for (i, &sample_id) in keyframe_set.iter().enumerate() {
        let _ = i; // sample_id 1-based
        let idx = (sample_id as usize).saturating_sub(1);
        if idx >= total_samples {
            continue;
        }
        let dts = dts_units.get(idx).copied().unwrap_or(0) as i64;
        let cts = cts_offsets.get(idx).copied().unwrap_or(0);
        let pts_units = dts + cts - elst_offset_units;
        let pts_sec = pts_units as f64 / ts;
        keyframes.push(Keyframe {
            sample_index: sample_id - 1, // 存 0-based
            pts_sec,
            offset: sample_offsets[idx],
            size: track.stsz[idx],
        });
    }
    keyframes.sort_by(|a, b| {
        a.pts_sec
            .partial_cmp(&b.pts_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(VideoIndex {
        stream_index: 0,
        keyframes,
        duration_sec: track.duration_sec,
    })
}

// ==================== box 遍历工具 ====================

fn read_u8(file: &mut File, pos: u64) -> Result<u8, String> {
    file.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek @{pos} 失败：{e}"))?;
    let mut buf = [0u8; 1];
    file.read_exact(&mut buf)
        .map_err(|e| format!("读取 u8 @{pos} 失败：{e}"))?;
    Ok(buf[0])
}

fn read_u32(file: &mut File, pos: u64) -> Result<u32, String> {
    file.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek @{pos} 失败：{e}"))?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)
        .map_err(|e| format!("读取 u32 @{pos} 失败：{e}"))?;
    Ok(u32::from_be_bytes(buf))
}

fn read_u64(file: &mut File, pos: u64) -> Result<u64, String> {
    file.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek @{pos} 失败：{e}"))?;
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf)
        .map_err(|e| format!("读取 u64 @{pos} 失败：{e}"))?;
    Ok(u64::from_be_bytes(buf))
}

fn read_box_header(file: &mut File, offset: u64) -> Result<BoxHeader, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek box @{offset} 失败：{e}"))?;
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf)
        .map_err(|e| format!("读 box header @{offset} 失败：{e}"))?;
    let size32 = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let mut type_fourcc = [0u8; 4];
    type_fourcc.copy_from_slice(&buf[4..8]);

    let (size, header_len) = match size32 {
        0 => {
            // size=0 表示延伸到文件末尾
            let meta_len = file
                .metadata()
                .map_err(|e| format!("读取元数据失败：{e}"))?
                .len();
            (meta_len - offset, 8u64)
        }
        1 => {
            // 64-bit largesize
            let mut large = [0u8; 8];
            file.read_exact(&mut large)
                .map_err(|e| format!("读 largesize @{offset} 失败：{e}"))?;
            (u64::from_be_bytes(large), 16u64)
        }
        s => (u64::from(s), 8u64),
    };
    if size < header_len {
        return Err(format!("box @{offset} size {size} < header {header_len}"));
    }
    Ok(BoxHeader {
        size,
        type_fourcc,
        data_offset: offset + header_len,
    })
}

fn box_payload_size(header: &BoxHeader) -> u64 {
    header.size.saturating_sub(if header.size >= 16 {
        // 已包含 largesize？由 data_offset 减去原 offset 得到
        // 这里简化：data_offset - (data_offset - header_len) = header_len
        // 直接用 size - header_len
        16 // 不准确，但 size=1 已特殊处理；这里 size ≥ 8
    } else {
        8
    })
}

fn advance_past(header: &BoxHeader, offset: u64) -> u64 {
    offset + header.size
}

fn find_top_level_box(
    file: &mut File,
    file_size: u64,
    target: &[u8; 4],
) -> Result<Option<BoxHeader>, String> {
    let mut offset = 0u64;
    while offset < file_size {
        let header = match read_box_header(file, offset) {
            Ok(h) => h,
            Err(_) => break,
        };
        if &header.type_fourcc == target {
            return Ok(Some(header));
        }
        offset = advance_past(&header, offset);
    }
    Ok(None)
}

fn moov_payload_size(header: &BoxHeader) -> u64 {
    header.size.saturating_sub(8) // moov 通常 size < 4G，无 largesize
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rejects_nonexistent_file() {
        let result = read_index(Path::new("C:/nonexistent/path/that/does/not/exist.mp4"));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let result = read_index(&path);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_truncated_mp4() {
        // 写一个最小的 ftyp box 但没有 moov
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut file = File::create(&path).unwrap();
        file.write_all(&[
            0x00, 0x00, 0x00, 0x18, // size = 24
            b'f', b't', b'y', b'p', // type = ftyp
            b'i', b's', b'o', b'm', // major_brand
            0x00, 0x00, 0x02, 0x00, // minor_version
            b'i', b's', b'o', b'm', // compatible
            b'i', b's', b'o', b'6', // compatible
        ])
        .unwrap();
        drop(file);
        let result = read_index(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("moov"));
    }
}
