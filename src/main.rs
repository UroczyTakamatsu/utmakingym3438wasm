use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use ymfm_sys::ffi::{self, ChipType};

const VGM_RATE: u64 = 44_100;
const FALLBACK_YM_CLOCK: u32 = 7_670_454;

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn vgm_bytes(path: &str) -> io::Result<Vec<u8>> {
    let raw = fs::read(path)?;
    println!("Input file: {path}");
    println!("Input size: {} bytes", raw.len());

    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut d = GzDecoder::new(&raw[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out)?;
        println!("VGZ decompressed size: {} bytes", out.len());
        Ok(out)
    } else {
        println!("Input is not gzip-compressed; treating it as VGM.");
        Ok(raw)
    }
}

fn write_wav(path: &str, samples: &[i16], channels: u16, rate: u32) -> io::Result<()> {
    if samples.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "renderer produced 0 PCM samples; refusing to create a 44-byte header-only WAV",
        ));
    }

    let data_bytes = (samples.len() * 2) as u32;
    let mut f = fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_bytes).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    let byte_rate = rate * channels as u32 * 2;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&(channels * 2).to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

fn command_size(bytes: &[u8], pos: usize, eof: usize) -> Option<usize> {
    if pos >= eof {
        return None;
    }
    let c = bytes[pos];
    let len = match c {
        0x30..=0x3f => 2,
        0x40..=0x4e => 3,
        0x4f..=0x50 => 2,
        0x51..=0x5f => 3,
        0x60 => 1,
        0x61 => 3,
        0x62..=0x66 => 1,
        0x67 => {
            if pos + 7 > eof || bytes[pos + 1] != 0x66 {
                return None;
            }
            let data_len = u32le(bytes, pos + 3) as usize;
            7usize.checked_add(data_len)?
        }
        0x68 => 12,
        0x69 => 1,
        0x6a => 1,
        0x70..=0x8f => 1,
        0x90..=0x91 => 5,
        0x92 => 6,
        0x93 => 11,
        0x94 => 2,
        0x95 => 5,
        0xa0..=0xbf => 3,
        0xc0..=0xdf => 4,
        0xe0..=0xff => 5,
        _ => return None,
    };
    if pos.checked_add(len)? <= eof { Some(len) } else { None }
}

fn render_wait(
    chip: &mut cxx::UniquePtr<ffi::Chip>,
    out: &mut Vec<i16>,
    channels: usize,
    native_rate: u32,
    wait_44100: u64,
    stats_peak: &mut i32,
) {
    if wait_44100 == 0 {
        return;
    }

    let n = ((wait_44100 as u128 * native_rate as u128) + 22_050) / 44_100;
    let n = n as usize;
    if n == 0 {
        return;
    }

    let mut buf = vec![0i32; n * channels];
    chip.pin_mut().generate(&mut buf);

    for x in buf {
        *stats_peak = (*stats_peak).max(x.abs());
        let y = (x >> 8).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        out.push(y);
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: ym3438-vgm-render INPUT.vgm|INPUT.vgz OUTPUT.wav");
        std::process::exit(2);
    }

    let bytes = vgm_bytes(&args[1])?;

    if bytes.len() < 0x40 || &bytes[0..4] != b"Vgm " {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a VGM file"));
    }

    let version = u32le(&bytes, 0x08);
    let data_off = if version >= 0x0001_0050 {
        let rel = u32le(&bytes, 0x34);
        if rel == 0 { 0x40 } else { 0x34 + rel as usize }
    } else {
        0x40
    };

    let eof_rel = u32le(&bytes, 0x04);
    let eof = if eof_rel == 0 {
        bytes.len()
    } else {
        (0x04 + eof_rel as usize).min(bytes.len())
    };

    let loop_rel = u32le(&bytes, 0x1c);
    let loop_pos = if loop_rel == 0 {
        None
    } else {
        Some(0x1c + loop_rel as usize)
    };

    let loop_samples = u32le(&bytes, 0x20);
    let clock_raw = u32le(&bytes, 0x2c);
    let ym_clock = clock_raw & 0x3fff_ffff;

    println!("VGM version: 0x{version:08X}");
    println!("VGM data offset: 0x{data_off:X}");
    println!("VGM EOF: 0x{eof:X}");
    println!("YM2612 clock in VGM: {ym_clock} Hz");
    println!("Loop position: {loop_pos:?}");
    println!("Loop samples: {loop_samples}");

    if data_off >= eof {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid VGM data offset"));
    }

    let clock = if ym_clock != 0 { ym_clock } else { FALLBACK_YM_CLOCK };
    let mut chip = ffi::create_chip(ChipType::Ym3438, clock);
    chip.pin_mut().reset();

    let channels = chip.channels() as usize;
    let native_rate = chip.sample_rate() as u32;

    println!("YM3438 clock used: {clock} Hz");
    println!("YM3438 output channels: {channels}");
    println!("YM3438 native sample rate: {native_rate} Hz");

    if channels == 0 || native_rate == 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "invalid YM3438 output format"));
    }

    // VGM PCM data blocks. Type 0x00 is the YM2612 DAC data bank used by 0x80..0x8F.
    let mut data_blocks: HashMap<u8, Vec<u8>> = HashMap::new();

    let mut out: Vec<i16> = Vec::new();
    let mut pos = data_off;
    let mut looped_once = false;
    let mut ended = false;

    let mut command_count = 0u64;
    let mut ym_writes = 0u64;
    let mut wait_44100 = 0u64;
    let mut data_block_count = 0u64;
    let mut dac_stream_writes = 0u64;
    let mut dac_pos = 0usize;
    let mut peak = 0i32;

    while !ended {
        if pos >= eof {
            if let Some(lp) = loop_pos {
                if !looped_once && lp < eof {
                    println!("Reached EOF; jumping to loop position 0x{lp:X}");
                    pos = lp;
                    looped_once = true;
                    continue;
                }
            }
            break;
        }

        let cmd = bytes[pos];
        command_count += 1;

        match cmd {
            0x50 => {
                // SN76489 write; ignored for this YM3438-only renderer.
                pos += 2;
            }

            0x52 | 0x53 => {
                if pos + 3 > eof {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated YM2612 write"));
                }
                let reg = bytes[pos + 1];
                let data = bytes[pos + 2];
                let port = if cmd == 0x52 { 0 } else { 2 };
                chip.pin_mut().write(port, reg);
                chip.pin_mut().write(port + 1, data);
                ym_writes += 1;
                pos += 3;
            }

            0x61 => {
                let n = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]) as u64;
                render_wait(&mut chip, &mut out, channels, native_rate, n, &mut peak);
                wait_44100 += n;
                pos += 3;
            }

            0x62 => {
                render_wait(&mut chip, &mut out, channels, native_rate, 735, &mut peak);
                wait_44100 += 735;
                pos += 1;
            }

            0x63 => {
                render_wait(&mut chip, &mut out, channels, native_rate, 882, &mut peak);
                wait_44100 += 882;
                pos += 1;
            }

            0x67 => {
                let len = command_size(&bytes, pos, eof).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid VGM data block")
                })?;
                let block_type = bytes[pos + 2];
                let data_len = u32le(&bytes, pos + 3) as usize;
                let start = pos + 7;
                let end = start + data_len;

                data_blocks.insert(block_type, bytes[start..end].to_vec());
                data_block_count += 1;

                println!(
                    "Data block #{data_block_count}: type=0x{block_type:02X}, size={data_len} bytes"
                );

                // YM2612 DAC stream normally uses data block type 0x00.
                if block_type == 0x00 {
                    dac_pos = 0;
                    println!("YM2612 DAC data bank loaded: {data_len} bytes");
                }

                pos += len;
            }

            0x68 => {
                // PCM RAM write command. Not needed for these YM2612 files.
                println!("Ignoring 0x68 PCM RAM write at 0x{pos:X}");
                pos += 12;
            }

            0x70..=0x7f => {
                let n = (cmd & 0x0f) as u64 + 1;
                render_wait(&mut chip, &mut out, channels, native_rate, n, &mut peak);
                wait_44100 += n;
                pos += 1;
            }

            0x80..=0x8f => {
                // YM2612 DAC data-bank write to register 0x2A, then wait n samples.
                // Per VGM spec, n is the low nibble (0..15), not n+1.
                let bank = data_blocks.get(&0x00).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "0x80..0x8F encountered before YM2612 DAC data block type 0x00",
                    )
                })?;

                if dac_pos >= bank.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "YM2612 DAC data bank exhausted",
                    ));
                }

                let sample = bank[dac_pos];
                dac_pos += 1;

                chip.pin_mut().write(0, 0x2a);
                chip.pin_mut().write(1, sample);
                ym_writes += 1;
                dac_stream_writes += 1;

                let n = (cmd & 0x0f) as u64;
                render_wait(&mut chip, &mut out, channels, native_rate, n, &mut peak);
                wait_44100 += n;
                pos += 1;
            }

            0xe0 => {
                let offset = u32le(&bytes, pos + 1) as usize;
                let bank = data_blocks.get(&0x00).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "0xE0 encountered before YM2612 DAC data block type 0x00",
                    )
                })?;

                if offset >= bank.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("0xE0 DAC seek offset {offset} exceeds bank size {}", bank.len()),
                    ));
                }

                dac_pos = offset;
                pos += 5;
            }

            0x66 => {
                if let Some(lp) = loop_pos {
                    if !looped_once && lp < eof {
                        println!("VGM end command: playing one loop pass from 0x{lp:X}");
                        pos = lp;
                        looped_once = true;
                        continue;
                    }
                }
                ended = true;
                pos += 1;
            }

            _ => {
                let len = command_size(&bytes, pos, eof).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported/invalid VGM command 0x{cmd:02X} at 0x{pos:X}"),
                    )
                })?;
                pos += len;
            }
        }
    }

    let frames = out.len() / channels;
    let duration = wait_44100 as f64 / VGM_RATE as f64;

    println!();
    println!("=== YM3438 render diagnostics ===");
    println!("Commands processed: {command_count}");
    println!("YM2612/YM3438 register writes: {ym_writes}");
    println!("YM2612 DAC stream writes (0x80..0x8F): {dac_stream_writes}");
    println!("Data blocks: {data_block_count}");
    println!("VGM wait samples: {wait_44100}");
    println!("Estimated VGM duration: {duration:.3} seconds");
    println!("Generated PCM frames: {frames}");
    println!("Generated PCM values: {}", out.len());
    println!("Peak raw YM3438 sample: {peak}");
    println!("Loop executed: {looped_once}");
    println!("=================================");
    println!();

    if frames == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "generated 0 PCM frames; WAV was not written",
        ));
    }

    write_wav(&args[2], &out, channels as u16, native_rate)?;

    let wav_size = fs::metadata(&args[2])?.len();
    println!("WAV written: {} ({} bytes)", args[2], wav_size);

    Ok(())
}
