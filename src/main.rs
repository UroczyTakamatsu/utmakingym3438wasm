use flate2::read::GzDecoder;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use ymfm_sys::ffi::{self, ChipType};

const VGM_RATE: u64 = 44_100;
const YM_CLOCK: u32 = 7_670_454;

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn vgm_bytes(path: &str) -> io::Result<Vec<u8>> {
    let raw = fs::read(path)?;
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut d = GzDecoder::new(&raw[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out)?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

fn write_wav(path: &str, samples: &[i16], channels: u16, rate: u32) -> io::Result<()> {
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
    let block_align = channels * 2;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
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
    println!("YM2612 clock in VGM: {ym_clock} Hz");
    println!("VGM data offset: 0x{data_off:X}");
    println!("loop offset: {:?}", loop_pos);
    println!("loop samples: {loop_samples}");

    if data_off >= eof {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid VGM data offset"));
    }

    let clock = if ym_clock != 0 { ym_clock } else { YM_CLOCK };
    let mut chip = ffi::create_chip(ChipType::Ym3438, clock);
    chip.pin_mut().reset();

    let channels = chip.channels() as usize;
    let native_rate = chip.sample_rate() as u32;
    if channels == 0 || native_rate == 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "invalid YM3438 output format"));
    }

    // Render at the chip's native rate. This first-stage renderer intentionally
    // keeps VGM timing in 44.1 kHz ticks and converts each wait to the required
    // number of native YM3438 samples.
    let mut out: Vec<i16> = Vec::new();
    let mut pending_wait: u64 = 0;
    let mut pos = data_off;
    let mut ended = false;
    let mut looped_once = false;

    fn render_wait(
        chip: &mut cxx::UniquePtr<ffi::Chip>,
        out: &mut Vec<i16>,
        channels: usize,
        native_rate: u32,
        wait_44100: u64,
    ) {
        if wait_44100 == 0 { return; }
        let n = ((wait_44100 as u128 * native_rate as u128) + 22_050) / 44_100;
        let n = n as usize;
        if n == 0 { return; }
        let mut buf = vec![0i32; n * channels];
        chip.pin_mut().generate(&mut buf);
        for x in buf {
            let y = (x >> 8).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            out.push(y);
        }
    }

    while !ended {
        if pos >= eof {
            if let Some(lp) = loop_pos {
                if !looped_once && lp < eof {
                    pos = lp;
                    looped_once = true;
                    continue;
                }
            }
            break;
        }

        let cmd = bytes[pos];
        match cmd {
            0x50 => {
                if pos + 1 >= eof { break; }
                pos += 2; // PSG is intentionally ignored in this YM3438-only renderer.
            }
            0x52 | 0x53 => {
                if pos + 2 >= eof { break; }
                let reg = bytes[pos + 1];
                let data = bytes[pos + 2];
                let port = if cmd == 0x52 { 0 } else { 2 };
                chip.pin_mut().write(port, reg);
                chip.pin_mut().write(port + 1, data);
                pos += 3;
            }
            0x61 => {
                if pos + 2 >= eof { break; }
                let n = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]) as u64;
                render_wait(&mut chip, &mut out, channels, native_rate, n);
                pos += 3;
            }
            0x62 => {
                render_wait(&mut chip, &mut out, channels, native_rate, 735);
                pos += 1;
            }
            0x63 => {
                render_wait(&mut chip, &mut out, channels, native_rate, 882);
                pos += 1;
            }
            0x66 => {
                if let Some(lp) = loop_pos {
                    if !looped_once && lp < eof {
                        pos = lp;
                        looped_once = true;
                        continue;
                    }
                }
                ended = true;
                pos += 1;
            }
            0x70..=0x7f => {
                render_wait(&mut chip, &mut out, channels, native_rate, (cmd & 0x0f) as u64 + 1);
                pos += 1;
            }
            _ => {
                // Handle common VGM commands enough to skip safely. Unsupported
                // commands are not sent to YM3438.
                let len = match cmd {
                    0x4f | 0x51 | 0x54 | 0x55 | 0x56 | 0x57 | 0x58 | 0x59 |
                    0x5a | 0x5b | 0x5c | 0x5d | 0x5e | 0x5f | 0x80..=0x8f => 0,
                    0x90..=0x95 => 5,
                    0xa0..=0xdf => 3,
                    0xe0 => 5,
                    _ => 1,
                };
                if len == 0 {
                    pos += 2;
                } else {
                    if pos + len > eof { break; }
                    pos += len;
                }
            }
        }
    }

    let output_channels = channels as u16;
    write_wav(&args[2], &out, output_channels, native_rate)?;
    println!("YM3438 render complete");
    println!("channels: {channels}");
    println!("sample rate: {native_rate}");
    println!("output samples: {}", out.len());
    println!("output frames: {}", out.len() / channels);
    println!("WAV: {}", args[2]);
    Ok(())
}
