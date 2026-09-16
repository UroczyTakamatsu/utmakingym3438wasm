use flate2::read::GzDecoder;
use std::io::Read;
use std::fs;
use ymfm_sys::ffi;
use std::collections::HashMap;

const VGM_RATE: u64 = 44_100;
const FALLBACK_YM_CLOCK: u32 = 7_670_454;
const OUTPUT_GAIN: i32 = 128; // Final PCM gain; YM3438 synthesis itself is unchanged.
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn load_vgm_input() -> Result<Vec<u8>, String> {
    const INPUT_PATH: &str = "/in/input.vgm";
    let input = fs::read(INPUT_PATH).map_err(|e| format!("failed to read {INPUT_PATH}: {e}"))?;
    if input.len() >= 2 && input[0] == 0x1f && input[1] == 0x8b {
        let mut d = GzDecoder::new(&input[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out).map_err(|e| format!("VGZ gzip decode failed: {e}"))?;
        Ok(out)
    } else {
        Ok(input)
    }
}

fn command_size(bytes: &[u8], pos: usize, eof: usize) -> Option<usize> {
    if pos >= eof { return None; }
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
            if pos + 7 > eof || bytes[pos + 1] != 0x66 { return None; }
            7usize.checked_add(u32le(bytes, pos + 3) as usize)?
        }
        0x68 => 12,
        0x69..=0x6a => 1,
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

fn render_wait(chip: &mut cxx::UniquePtr<ffi::Chip>, out: &mut Vec<i16>, channels: usize, native_rate: u32, wait: u64, peak: &mut i32) -> u64 {
    if wait == 0 { return 0; }
    let n = ((wait as u128 * native_rate as u128) + 22_050) / 44_100;
    let n = n as usize;
    if n == 0 { return 0; }
    let mut buf = vec![0i32; n * channels];
    chip.pin_mut().generate(&mut buf);
    for x in buf {
        *peak = (*peak).max(x.abs());
        let y = ((x >> 8) * OUTPUT_GAIN).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        out.push(y);
    }
    n as u64
}

fn main() {
    if let Err(e)=run() { eprintln!("ERROR: {e}"); std::process::exit(2); }
}

fn run() -> Result<(), String> {
    println!("YM3438 browser VGM/VGZ -> PCM diagnostic test");
    let input_meta = fs::metadata("/in/input.vgm").map_err(|e| format!("input file metadata failed: {e}"))?;
    println!("input_bytes={}", input_meta.len());
    let bytes=load_vgm_input()?;
    println!("vgm_decompressed_bytes={}", bytes.len());
    if bytes.len()<0x40 || &bytes[0..4]!=b"Vgm " { return Err("not a VGM file".into()); }
    let version=u32le(&bytes,0x08);
    let data_off=if version>=0x0001_0050 { let r=u32le(&bytes,0x34); if r==0 {0x40} else {0x34+r as usize} } else {0x40};
    let eof_rel=u32le(&bytes,0x04);
    let eof=if eof_rel==0 {bytes.len()} else {(0x04+eof_rel as usize).min(bytes.len())};
    let clock_raw=u32le(&bytes,0x2c);
    let clock=if clock_raw & 0x3fff_ffff != 0 {clock_raw & 0x3fff_ffff} else {FALLBACK_YM_CLOCK};
    println!("vgm_version=0x{version:08X}");
    println!("vgm_data_offset=0x{data_off:X}");
    println!("ym2612_clock={clock}");
    if data_off>=eof {return Err("invalid VGM data offset".into());}
    let mut chip=ffi::create_chip(ffi::ChipType::Ym3438,clock);
    chip.pin_mut().reset();
    let channels=chip.channels() as usize; let rate=chip.sample_rate() as u32;
    println!("ym3438_channels={channels}"); println!("ym3438_native_rate={rate}");
    let loop_rel=u32le(&bytes,0x1c);
    let loop_pos=if loop_rel==0 {None} else {Some(0x1c + loop_rel as usize)};
    let loop_samples=u32le(&bytes,0x20);
    println!("vgm_loop_position={loop_pos:?}");
    println!("vgm_loop_samples={loop_samples}");
    let mut blocks:HashMap<u8,Vec<u8>>=HashMap::new();
    let mut out=Vec::<i16>::new(); let mut pos=data_off; let mut dac_pos=0usize; let mut waits=0u64; let mut writes=0u64; let mut dac_writes=0u64; let mut ym_writes=0u64; let mut data_blocks=0u64; let mut commands=0u64; let mut peak=0i32; let mut ended=false; let mut looped_once=false;
    while !ended {
        if pos>=eof {
            if let Some(lp)=loop_pos {
                if !looped_once && lp < eof { pos=lp; looped_once=true; continue; }
            }
            break;
        }
        let cmd=bytes[pos]; commands+=1;
        match cmd {
            0x50 => pos+=2,
            0x52|0x53 => { if pos+3>eof {return Err("truncated YM write".into());} let reg=bytes[pos+1]; let val=bytes[pos+2]; let port=if cmd==0x52 {0} else {2}; chip.pin_mut().write(port,reg); chip.pin_mut().write(port+1,val); writes+=1; ym_writes+=1; pos+=3; }
            0x61 => {let n=u16::from_le_bytes([bytes[pos+1],bytes[pos+2]]) as u64; render_wait(&mut chip,&mut out,channels,rate,n,&mut peak); waits+=n; pos+=3;}
            0x62 => {render_wait(&mut chip,&mut out,channels,rate,735,&mut peak); waits+=735; pos+=1;}
            0x63 => {render_wait(&mut chip,&mut out,channels,rate,882,&mut peak); waits+=882; pos+=1;}
            0x67 => {let len=command_size(&bytes,pos,eof).ok_or("invalid 0x67 data block")?; let ty=bytes[pos+2]; let n=u32le(&bytes,pos+3) as usize; let st=pos+7; blocks.insert(ty,bytes[st..st+n].to_vec()); data_blocks+=1; if ty==0 {dac_pos=0;} pos+=len;}
            0x70..=0x7f => {let n=(cmd&0x0f) as u64+1; render_wait(&mut chip,&mut out,channels,rate,n,&mut peak); waits+=n; pos+=1;}
            0x80..=0x8f => {let bank=blocks.get(&0).ok_or("DAC bank missing")?; if dac_pos>=bank.len() {return Err("DAC bank exhausted".into());} let v=bank[dac_pos]; dac_pos+=1; chip.pin_mut().write(0,0x2a); chip.pin_mut().write(1,v); writes+=1; dac_writes+=1; let n=(cmd&0x0f) as u64; render_wait(&mut chip,&mut out,channels,rate,n,&mut peak); waits+=n; pos+=1;}
            0xe0 => {let off=u32le(&bytes,pos+1) as usize; let bank=blocks.get(&0).ok_or("DAC bank missing")?; if off>=bank.len() {return Err("DAC seek out of range".into());} dac_pos=off; pos+=5;}
            0x66 => {
                if let Some(lp)=loop_pos {
                    if !looped_once && lp < eof {
                        println!("vgm_end_reached=true; playing one loop pass from 0x{lp:X}");
                        pos=lp; looped_once=true; continue;
                    }
                }
                ended=true;
            }
            _ => {let len=command_size(&bytes,pos,eof).ok_or_else(||format!("unsupported command 0x{cmd:02X} at 0x{pos:X}"))?; pos+=len;}
        }
    }
    let frames=out.len()/channels;
    println!("commands={commands}"); println!("register_writes={writes}"); println!("ym_register_writes={ym_writes}"); println!("dac_writes={dac_writes}"); println!("data_blocks={data_blocks}"); println!("wait_samples={waits}"); println!("duration_seconds={:.3}", waits as f64 / VGM_RATE as f64); println!("generated_frames={frames}"); println!("peak_raw={peak}"); println!("output_gain={OUTPUT_GAIN}"); println!("looped_once={looped_once}");
    let preview: Vec<String> = out.iter().take(32).map(|v| v.to_string()).collect();
    println!("pcm_preview={}", preview.join(","));
    if frames==0 || peak==0 {return Err("VGM parsing completed but PCM is silent".into());}
    const OUTPUT_PATH: &str = "/out/ym3438_output.pcm";
    let mut pcm_bytes = Vec::with_capacity(out.len() * 2);
    for s in &out { pcm_bytes.extend_from_slice(&s.to_le_bytes()); }
    fs::write(OUTPUT_PATH, &pcm_bytes).map_err(|e| format!("failed to write {OUTPUT_PATH}: {e}"))?;
    println!("pcm_path={OUTPUT_PATH}");
    println!("pcm_bytes={}", pcm_bytes.len());
    println!("pcm_channels={channels}");
    println!("pcm_sample_rate={rate}");
    Ok(())
}
