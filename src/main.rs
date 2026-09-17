use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use ymfm_sys::ffi;

const VGM_RATE: u64 = 44_100;
const FALLBACK_YM_CLOCK: u32 = 7_670_454;
const OUTPUT_GAIN: i32 = 128;
const PSG_OUTPUT_GAIN: i32 = 1;
const PSG_LEVEL_SCALE: f64 = 1024.0;
const PSG_CLOCK_SCALE: f64 = 1.0;
const FM_STREAMS: usize = 7; // FM1..FM6 + DAC
const PSG_STREAMS: usize = 3;
const STREAMS: usize = FM_STREAMS + PSG_STREAMS; // FM1..FM6 + DAC + PSG1..PSG3
const FALLBACK_PSG_CLOCK: u32 = 3_579_545;

fn u32le(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }

fn load_vgm_input() -> Result<Vec<u8>, String> {
    let input = fs::read("/in/input.vgm").map_err(|e| format!("failed to read /in/input.vgm: {e}"))?;
    if input.len() >= 2 && input[0] == 0x1f && input[1] == 0x8b {
        let mut d = GzDecoder::new(&input[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out).map_err(|e| format!("VGZ gzip decode failed: {e}"))?;
        Ok(out)
    } else { Ok(input) }
}

fn command_size(bytes: &[u8], pos: usize, eof: usize) -> Option<usize> {
    if pos >= eof { return None; }
    let c = bytes[pos];
    let len = match c {
        0x30..=0x3f => 2, 0x40..=0x4e => 3, 0x4f..=0x50 => 2,
        0x51..=0x5f => 3, 0x60 => 1, 0x61 => 3, 0x62..=0x66 => 1,
        0x67 => { if pos + 7 > eof || bytes[pos + 1] != 0x66 { return None; } 7usize.checked_add(u32le(bytes, pos + 3) as usize)? }
        0x68 => 12, 0x69..=0x6a => 1, 0x70..=0x8f => 1,
        0x90..=0x91 => 5, 0x92 => 6, 0x93 => 11, 0x94 => 2, 0x95 => 5,
        0xa0..=0xbf => 3, 0xc0..=0xdf => 4, 0xe0..=0xff => 5,
        _ => return None,
    };
    if pos.checked_add(len)? <= eof { Some(len) } else { None }
}

struct TimingAccumulator { remainder: u128 }
impl TimingAccumulator {
    fn new() -> Self { Self { remainder: 0 } }
    fn frames_for_wait(&mut self, wait_samples: u64, native_rate: u32) -> usize {
        self.remainder += wait_samples as u128 * native_rate as u128;
        let frames = self.remainder / VGM_RATE as u128;
        self.remainder %= VGM_RATE as u128;
        frames as usize
    }
}



struct PsgChip {
    clock: u32,
    rate: u32,
    tone: [u16; 3],
    volume: [u8; 4],
    noise: u8,
    latched: u8,
    phase: [f64; 3],
    noise_phase: f64,
    lfsr: u16,
    out: [Vec<i16>; 3],
}

impl PsgChip {
    fn new(clock: u32, rate: u32) -> Self {
        Self {
            clock: if clock == 0 { FALLBACK_PSG_CLOCK } else { clock }, rate,
            tone: [0; 3], volume: [15; 4], noise: 0, latched: 0,
            phase: [0.0; 3], noise_phase: 0.0, lfsr: 0x4000,
            out: [Vec::new(), Vec::new(), Vec::new()],
        }
    }

    fn write(&mut self, data: u8) {
        if data & 0x80 != 0 {
            self.latched = (data >> 4) & 0x07;
            let ch = (self.latched >> 1) as usize;
            let kind = self.latched & 1;
            let nibble = data & 0x0f;
            if ch < 3 {
                if kind == 0 {
                    self.tone[ch] = (self.tone[ch] & 0x3f0) | nibble as u16;
                } else {
                    self.volume[ch] = nibble;
                }
            } else if kind == 0 {
                self.noise = nibble & 0x07;
                self.lfsr = 0x4000;
            } else {
                self.volume[3] = nibble;
            }
        } else {
            let ch = (self.latched >> 1) as usize;
            let kind = self.latched & 1;
            if ch < 3 && kind == 0 {
                self.tone[ch] = (self.tone[ch] & 0x00f) | (((data & 0x3f) as u16) << 4);
            }
        }
    }

    fn tone_step(&self, ch: usize) -> f64 {
        let period = self.tone[ch].max(1) as f64;
        self.clock as f64 * PSG_CLOCK_SCALE / (32.0 * period) / self.rate as f64
    }

    fn noise_step(&self) -> f64 {
        let base = match self.noise & 0x03 {
            0 => self.clock as f64 * PSG_CLOCK_SCALE / 512.0,
            1 => self.clock as f64 * PSG_CLOCK_SCALE / 1024.0,
            2 => self.clock as f64 * PSG_CLOCK_SCALE / 2048.0,
            _ => self.clock as f64 * PSG_CLOCK_SCALE / (32.0 * self.tone[2].max(1) as f64),
        };
        base / self.rate as f64
    }

    fn level(volume: u8) -> f64 {
        if volume >= 15 { 0.0 } else { 1.0 - (volume as f64 / 15.0) }
    }

    fn generate(&mut self, frames: usize, skip: usize, gain: i32) {
        for i in 0..frames {
            for ch in 0..3 {
                self.phase[ch] += self.tone_step(ch);
                if self.phase[ch] >= 1.0 { self.phase[ch] -= self.phase[ch].floor(); }
            }
            self.noise_phase += self.noise_step();
            while self.noise_phase >= 1.0 {
                self.noise_phase -= 1.0;
                let white = (self.noise & 0x04) != 0;
                let feedback = if white {
                    (self.lfsr ^ (self.lfsr >> 1)) & 1
                } else {
                    (self.lfsr >> 14) & 1
                };
                self.lfsr = (self.lfsr >> 1) | (feedback << 14);
                if self.lfsr == 0 { self.lfsr = 0x4000; }
            }
            if i < skip { continue; }
            for ch in 0..3 {
                let sample = if ch == 2 && (self.noise & 0x03) != 0x03 {
                    if self.lfsr & 1 != 0 { Self::level(self.volume[3]) } else { -Self::level(self.volume[3]) }
                } else if self.phase[ch] < 0.5 {
                    Self::level(self.volume[ch])
                } else {
                    -Self::level(self.volume[ch])
                };
                let v = (sample * PSG_LEVEL_SCALE * gain as f64).clamp(i16::MIN as f64, i16::MAX as f64) as i16;
                // AudioWorklet expects every stream to be stereo interleaved,
                // just like the YM3438 streams. PSG itself is mono, so duplicate
                // each channel sample to L/R. This also keeps one PSG sample =
                // one native-rate frame; storing mono here made the worklet read
                // two PSG samples as one stereo frame, causing 2x playback speed.
                self.out[ch].push(v);
                self.out[ch].push(v);
            }
        }
    }
}

struct StreamChip { chip: cxx::UniquePtr<ffi::Chip>, out: Vec<i16> }

fn make_chip(clock: u32, stream: usize) -> Result<StreamChip, String> {
    let mut chip = ffi::create_chip(ffi::ChipType::Ym3438, clock);
    chip.pin_mut().reset();
    // Default to stereo output for the selected FM channel. Non-selected
    // channels are muted by clearing their L/R panning bits below.
    for ch in 0..6u32 {
        let reg = 0xb4 + ch;
        let value = if stream < 6 && ch as usize == stream { 0xc0 } else { 0x00 };
        let port = if ch >= 3 { 2 } else { 0 };
        chip.pin_mut().write(port, (reg - if ch >= 3 { 3 } else { 0 }) as u8);
        chip.pin_mut().write(port + 1, value);
    }
    // FM streams must not contain DAC; DAC stream forces DAC on.
    chip.pin_mut().write(0, 0x2b);
    chip.pin_mut().write(1, if stream == 6 { 0x01 } else { 0x00 });
    Ok(StreamChip { chip, out: Vec::new() })
}

fn apply_ym_write(sc: &mut StreamChip, stream: usize, cmd: u8, reg: u8, val: u8) {
    let port = if cmd == 0x52 { 0 } else { 2 };
    sc.chip.pin_mut().write(port, reg);
    let mut value = val;
    // B4-B6 are channel pan/output registers. Keep the original L/R routing
    // for the selected stream and mute the other FM channels.
    if (0xb4..=0xb6).contains(&reg) {
        let ch = if cmd == 0x52 { (reg - 0xb4) as usize } else { (reg - 0xb4 + 3) as usize };
        let selected = stream < 6 && ch == stream;
        value = if selected { val | 0x00 } else { val & 0x3f };
        // For a selected stream preserve the original pan bits. For safety,
        // if neither side is selected, enable both so a channel remains audible.
        if selected && (value & 0xc0) == 0 { value |= 0xc0; }
    }
    // DAC enable (2B bit 0): force it off for FM streams and on for DAC.
    if cmd == 0x52 && reg == 0x2b { value = if stream == 6 { val | 1 } else { val & !1 }; }
    sc.chip.pin_mut().write(port + 1, value);
}

fn render_wait(streams: &mut [StreamChip], channels: usize, native_rate: u32, wait: u64,
              timing: &mut TimingAccumulator, skip_frames: &mut u64, peak: &mut i32) -> (u64, usize) {
    if wait == 0 { return (0, 0); }
    let n = timing.frames_for_wait(wait, native_rate);
    if n == 0 { return (0, 0); }
    let skip = (*skip_frames).min(n as u64) as usize;
    for sc in streams.iter_mut() {
        let mut buf = vec![0i32; n * channels];
        sc.chip.pin_mut().generate(&mut buf);
        for &x in &buf { *peak = (*peak).max(x.abs()); }
        for &x in &buf[skip * channels..] {
            sc.out.push(((x >> 8) * OUTPUT_GAIN).clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        }
    }
    *skip_frames -= skip as u64;
    (n as u64, skip)
}

fn main() { if let Err(e) = run() { eprintln!("ERROR: {e}"); std::process::exit(2); } }

fn run() -> Result<(), String> {
    let seek_ms: u64 = env::args().skip(1).find_map(|a| a.strip_prefix("--seek-ms=").and_then(|v| v.parse().ok())).unwrap_or(0);
    println!("YM3438 browser VGM/VGZ -> per-channel PCM diagnostic test");
    println!("seek_ms={seek_ms}");
    println!("channel_streams=FM1,FM2,FM3,FM4,FM5,FM6,DAC,PSG1,PSG2,PSG3");
    let input_meta = fs::metadata("/in/input.vgm").map_err(|e| format!("input file metadata failed: {e}"))?;
    println!("input_bytes={}", input_meta.len());
    let bytes = load_vgm_input()?;
    println!("vgm_decompressed_bytes={}", bytes.len());
    if bytes.len() < 0x40 || &bytes[0..4] != b"Vgm " { return Err("not a VGM file".into()); }
    let version = u32le(&bytes, 0x08);
    let data_off = if version >= 0x0001_0050 { let r=u32le(&bytes,0x34); if r==0 {0x40} else {0x34+r as usize} } else {0x40};
    let eof_rel=u32le(&bytes,0x04); let eof=if eof_rel==0 {bytes.len()} else {(0x04+eof_rel as usize).min(bytes.len())};
    let clock_raw=u32le(&bytes,0x2c); let clock=if clock_raw&0x3fff_ffff!=0 {clock_raw&0x3fff_ffff} else {FALLBACK_YM_CLOCK};
    let psg_clock_raw=u32le(&bytes,0x0c); let psg_clock=if psg_clock_raw&0x3fff_ffff!=0 {psg_clock_raw&0x3fff_ffff} else {FALLBACK_PSG_CLOCK};
    println!("vgm_version=0x{version:08X}"); println!("vgm_data_offset=0x{data_off:X}"); println!("ym2612_clock={clock}"); println!("sn76489_clock={psg_clock}"); println!("psg_level_scale={PSG_LEVEL_SCALE}");
    if data_off>=eof {return Err("invalid VGM data offset".into());}
    let probe=ffi::create_chip(ffi::ChipType::Ym3438,clock); let channels=probe.channels() as usize; let rate=probe.sample_rate() as u32;
    println!("ym3438_channels={channels}"); println!("ym3438_native_rate={rate}"); println!("psg_output_gain={PSG_OUTPUT_GAIN}"); println!("psg_clock_scale={PSG_CLOCK_SCALE}");
    let target_frames=((seek_ms as u128*rate as u128)/1000) as u64; let mut skip_frames=target_frames;
    println!("seek_target_native_frames={target_frames}");
    let loop_rel=u32le(&bytes,0x1c); let loop_pos=if loop_rel==0{None}else{Some(0x1c+loop_rel as usize)}; let loop_samples_header=u32le(&bytes,0x20);
    println!("vgm_loop_position={loop_pos:?}"); println!("vgm_loop_samples={loop_samples_header}");
    let mut streams=Vec::with_capacity(FM_STREAMS); for s in 0..FM_STREAMS { streams.push(make_chip(clock,s)?); }
    let mut psg=PsgChip::new(psg_clock,rate);
    let mut blocks:HashMap<u8,Vec<u8>>=HashMap::new(); let mut pos=data_off; let mut timeline_samples=0u64; let mut loop_start_samples=None; let mut loop_end_samples=None;
    let mut timing=TimingAccumulator::new(); let mut dac_pos=0usize; let mut waits=0u64; let mut writes=0u64; let mut dac_writes=0u64; let mut ym_writes=0u64; let mut data_blocks=0u64; let mut commands=0u64; let mut peak=0i32; let mut ended=false;
    while !ended {
        if pos >= eof { break; }
        let cmd=bytes[pos]; commands+=1;
        if Some(pos)==loop_pos && loop_start_samples.is_none(){loop_start_samples=Some(timeline_samples);}
        match cmd {
            0x50=>{ if pos+2>eof{return Err("truncated PSG write".into());} psg.write(bytes[pos+1]); pos+=2; },
            0x52|0x53=>{if pos+3>eof{return Err("truncated YM write".into());} let reg=bytes[pos+1];let val=bytes[pos+2]; for (s,sc) in streams.iter_mut().enumerate(){apply_ym_write(sc,s,cmd,reg,val);} writes+=1;ym_writes+=1;pos+=3;}
            0x61=>{let n=u16::from_le_bytes([bytes[pos+1],bytes[pos+2]]) as u64;let (generated_frames,skip)=render_wait(&mut streams,channels,rate,n,&mut timing,&mut skip_frames,&mut peak);psg.generate(generated_frames as usize,skip,PSG_OUTPUT_GAIN);waits+=n;timeline_samples+=n;pos+=3;}
            0x62=>{let (generated_frames,skip)=render_wait(&mut streams,channels,rate,735,&mut timing,&mut skip_frames,&mut peak);psg.generate(generated_frames as usize,skip,PSG_OUTPUT_GAIN);waits+=735;timeline_samples+=735;pos+=1;}
            0x63=>{let (generated_frames,skip)=render_wait(&mut streams,channels,rate,882,&mut timing,&mut skip_frames,&mut peak);psg.generate(generated_frames as usize,skip,PSG_OUTPUT_GAIN);waits+=882;timeline_samples+=882;pos+=1;}
            0x67=>{let len=command_size(&bytes,pos,eof).ok_or("invalid 0x67 data block")?;let ty=bytes[pos+2];let n=u32le(&bytes,pos+3) as usize;let st=pos+7;if st+n>eof{return Err("truncated 0x67 data block".into());}blocks.insert(ty,bytes[st..st+n].to_vec());data_blocks+=1;if ty==0{dac_pos=0;}pos+=len;}
            0x70..=0x7f=>{let n=(cmd&0x0f) as u64+1;let (generated_frames,skip)=render_wait(&mut streams,channels,rate,n,&mut timing,&mut skip_frames,&mut peak);psg.generate(generated_frames as usize,skip,PSG_OUTPUT_GAIN);waits+=n;timeline_samples+=n;pos+=1;}
            0x80..=0x8f=>{let bank=blocks.get(&0).ok_or("DAC bank missing")?;if dac_pos>=bank.len(){return Err("DAC bank exhausted".into());}let v=bank[dac_pos];dac_pos+=1;for (s,sc) in streams.iter_mut().enumerate(){apply_ym_write(sc,s,0x52,0x2a,v);}writes+=1;dac_writes+=1;let n=(cmd&0x0f) as u64;let (generated_frames,skip)=render_wait(&mut streams,channels,rate,n,&mut timing,&mut skip_frames,&mut peak);psg.generate(generated_frames as usize,skip,PSG_OUTPUT_GAIN);waits+=n;timeline_samples+=n;pos+=1;}
            0xe0=>{let off=u32le(&bytes,pos+1) as usize;let bank=blocks.get(&0).ok_or("DAC bank missing")?;if off>=bank.len(){return Err("DAC seek out of range".into());}dac_pos=off;pos+=5;}
            0x66=>{if loop_start_samples.is_some(){loop_end_samples=Some(timeline_samples);}ended=true;}
            _=>{let len=command_size(&bytes,pos,eof).ok_or_else(||format!("unsupported command 0x{cmd:02X} at 0x{pos:X}"))?;pos+=len;}
        }
    }
    let frames=streams.first().map(|s|s.out.len()/channels).unwrap_or(0); let timeline_duration=waits as f64/VGM_RATE as f64;
    let loop_start_seconds=loop_start_samples.map(|v|v as f64/VGM_RATE as f64); let loop_end_seconds=loop_end_samples.map(|v|v as f64/VGM_RATE as f64);
    println!("commands={commands}");println!("register_writes={writes}");println!("ym_register_writes={ym_writes}");println!("dac_writes={dac_writes}");println!("data_blocks={data_blocks}");println!("wait_samples={waits}");println!("duration_seconds={timeline_duration:.6}");println!("timeline_duration_seconds={timeline_duration:.6}");
    println!("loop_start_seconds={}",loop_start_seconds.map(|v|format!("{v:.6}")).unwrap_or("none".into()));println!("loop_end_seconds={}",loop_end_seconds.map(|v|format!("{v:.6}")).unwrap_or("none".into()));println!("loop_prepared={}",loop_start_seconds.is_some()&&loop_end_seconds.is_some());
    println!("generated_frames={frames}");println!("skipped_native_frames={}",target_frames.saturating_sub(skip_frames));println!("seek_effective_seconds={:.6}",target_frames as f64/rate as f64);println!("peak_raw={peak}");println!("output_gain={OUTPUT_GAIN}");println!("timing_remainder_1_44100={}",timing.remainder);
    if frames==0||peak==0{return Err("VGM parsing completed but PCM is silent".into());}
    let out_dir="/out"; for (s,sc) in streams.iter().enumerate(){let name=if s<6{format!("ym3438_fm{}.pcm",s+1)}else{"ym3438_dac.pcm".into()};let path=format!("{out_dir}/{name}");let mut pcm=Vec::with_capacity(sc.out.len()*2);for v in &sc.out{pcm.extend_from_slice(&v.to_le_bytes());}fs::write(&path,&pcm).map_err(|e|format!("failed to write {path}: {e}"))?;println!("stream{}_path={path}",s+1);println!("stream{}_bytes={}",s+1,pcm.len());} for ch in 0..3 { let path=format!("{out_dir}/psg{}.pcm",ch+1); let mut pcm=Vec::with_capacity(psg.out[ch].len()*2); for v in &psg.out[ch]{pcm.extend_from_slice(&v.to_le_bytes());} fs::write(&path,&pcm).map_err(|e|format!("failed to write {path}: {e}"))?; println!("psg{}_path={path}",ch+1); println!("psg{}_bytes={}",ch+1,pcm.len());}
    println!("pcm_channels={channels}");println!("pcm_sample_rate={rate}");println!("pcm_streams={STREAMS}");println!("psg_streams=3");Ok(())
}
