use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use ymfm_sys::ffi;

const VGM_RATE: u64 = 44_100;
const FALLBACK_YM_CLOCK: u32 = 7_670_454;
const OUTPUT_GAIN: i32 = 128;

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

fn render_wait(
    chip: &mut cxx::UniquePtr<ffi::Chip>,
    out: &mut Vec<i16>,
    channels: usize,
    native_rate: u32,
    wait: u64,
    timing: &mut TimingAccumulator,
    peak: &mut i32,
    skip_frames: &mut u64,
) -> u64 {
    if wait == 0 { return 0; }
    let n = timing.frames_for_wait(wait, native_rate);
    if n == 0 { return 0; }
    let mut buf = vec![0i32; n * channels];
    chip.pin_mut().generate(&mut buf);
    for x in &buf { *peak = (*peak).max(x.abs()); }

    let skip = (*skip_frames).min(n as u64) as usize;
    *skip_frames -= skip as u64;
    for &x in &buf[skip * channels..] {
        let y = ((x >> 8) * OUTPUT_GAIN).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        out.push(y);
    }
    n as u64
}


#[derive(Clone, Copy)]
enum RenderPart {
    Fm(usize),
    Dac,
}

fn write_ym(chip: &mut cxx::UniquePtr<ffi::Chip>, port: u32, reg: u8, val: u8, part: RenderPart) {
    let pass = match part {
        RenderPart::Fm(ch) => Some(ch),
        RenderPart::Dac => None,
    };

    // Global YM3438/YM2612 registers are needed by every isolated render.
    let global = matches!(reg, 0x22 | 0x24..=0x27 | 0x2b);
    let key_on = reg == 0x28;
    let channel_from_reg = |r: u8| -> Option<usize> {
        if (0x30..=0x9f).contains(&r) || (0xa0..=0xb6).contains(&r) {
            let c = (r & 0x03) as usize;
            if c < 3 { Some((port as usize / 2) * 3 + c) } else { None }
        } else { None }
    };

    match part {
        RenderPart::Fm(ch) => {
            let allow = if key_on {
                // 0x28 low bits select channel. Upper bits select operators.
                let c = ((val & 0x03) as usize) + if val & 0x04 != 0 { 3 } else { 0 };
                c == ch
            } else if global {
                true
            } else if let Some(c) = channel_from_reg(reg) {
                c == ch
            } else {
                // Other writes (including DAC register writes) do not belong to an FM channel.
                false
            };
            if allow {
                chip.pin_mut().write(port, reg);
                chip.pin_mut().write(port + 1, val);
            }
        }
        RenderPart::Dac => {
            if reg == 0x2a || reg == 0x2b || global {
                chip.pin_mut().write(port, reg);
                chip.pin_mut().write(port + 1, val);
            }
        }
    }
}

fn render_part(
    bytes: &[u8], data_off: usize, eof: usize, clock: u32, rate: u32,
    seek_frames_target: u64, part: RenderPart, label: &str,
) -> Result<(Vec<i16>, u64, i32, u64, u64, u64, u64), String> {
    let mut chip = ffi::create_chip(ffi::ChipType::Ym3438, clock);
    chip.pin_mut().reset();
    let channels = chip.channels() as usize;
    if channels != 2 { return Err(format!("unexpected YM3438 channel count: {channels}")); }
    let mut blocks: HashMap<u8, Vec<u8>> = HashMap::new();
    let mut timeline_samples = 0u64;
    let mut timing = TimingAccumulator::new();
    let mut out = Vec::<i16>::new();
    let mut pos = data_off;
    let mut dac_pos = 0usize;
    let mut skip_frames = seek_frames_target;
    let mut waits = 0u64;
    let mut writes = 0u64;
    let mut peak = 0i32;
    let mut commands = 0u64;
    let mut dac_writes = 0u64;
    let mut ym_writes = 0u64;
    let mut data_blocks = 0u64;
    let mut ended = false;

    while !ended {
        if pos >= eof { break; }
        let cmd = bytes[pos]; commands += 1;
        match cmd {
            0x50 => pos += 2,
            0x52 | 0x53 => {
                if pos + 3 > eof { return Err("truncated YM write".into()); }
                let reg = bytes[pos + 1]; let val = bytes[pos + 2];
                let port = if cmd == 0x52 { 0 } else { 2 };
                write_ym(&mut chip, port, reg, val, part);
                writes += 1; ym_writes += 1; pos += 3;
            }
            0x61 => {
                if pos + 3 > eof { return Err("truncated 0x61".into()); }
                let n = u16::from_le_bytes([bytes[pos+1], bytes[pos+2]]) as u64;
                render_wait(&mut chip, &mut out, channels, rate, n, &mut timing, &mut peak, &mut skip_frames);
                waits += n; timeline_samples += n; pos += 3;
            }
            0x62 => { render_wait(&mut chip,&mut out,channels,rate,735,&mut timing,&mut peak,&mut skip_frames); waits+=735; timeline_samples+=735; pos+=1; }
            0x63 => { render_wait(&mut chip,&mut out,channels,rate,882,&mut timing,&mut peak,&mut skip_frames); waits+=882; timeline_samples+=882; pos+=1; }
            0x67 => {
                let len=command_size(bytes,pos,eof).ok_or("invalid 0x67 data block")?;
                let ty=bytes[pos+2]; let n=u32le(bytes,pos+3) as usize; let st=pos+7;
                if st+n>eof{return Err("truncated 0x67 data block".into());}
                blocks.insert(ty,bytes[st..st+n].to_vec()); data_blocks+=1; if ty==0{dac_pos=0;} pos+=len;
            }
            0x70..=0x7f => { let n=(cmd&0x0f) as u64+1; render_wait(&mut chip,&mut out,channels,rate,n,&mut timing,&mut peak,&mut skip_frames); waits+=n; timeline_samples+=n; pos+=1; }
            0x80..=0x8f => {
                let bank=blocks.get(&0).ok_or("DAC bank missing")?;
                if dac_pos>=bank.len(){return Err("DAC bank exhausted".into());}
                let v=bank[dac_pos]; dac_pos+=1;
                // DAC writes are isolated from the six FM channels.
                if matches!(part,RenderPart::Dac) { chip.pin_mut().write(0,0x2a); chip.pin_mut().write(1,v); }
                writes+=1; dac_writes+=1;
                let n=(cmd&0x0f) as u64; render_wait(&mut chip,&mut out,channels,rate,n,&mut timing,&mut peak,&mut skip_frames); waits+=n; timeline_samples+=n; pos+=1;
            }
            0xe0 => {
                if pos+5>eof{return Err("truncated DAC seek".into());}
                let off=u32le(bytes,pos+1) as usize; let bank=blocks.get(&0).ok_or("DAC bank missing")?;
                if off>=bank.len(){return Err("DAC seek out of range".into());} dac_pos=off; pos+=5;
            }
            0x66 => { ended=true; }
            _ => { let len=command_size(bytes,pos,eof).ok_or_else(||format!("unsupported command 0x{cmd:02X} at 0x{pos:X}"))?; pos+=len; }
        }
    }
    println!("[PART {label}] commands={commands} ym_writes={ym_writes} dac_writes={dac_writes} waits={waits} frames={}", out.len()/2);
    Ok((out, timeline_samples, peak, commands, ym_writes, dac_writes, data_blocks))
}

fn main() { if let Err(e)=run(){ eprintln!("ERROR: {e}"); std::process::exit(2); } }

fn run() -> Result<(), String> {
    let seek_ms: u64 = env::args().skip(1).find_map(|a|a.strip_prefix("--seek-ms=").and_then(|v|v.parse().ok())).unwrap_or(0);
    println!("YM3438 browser VGM/VGZ -> PCM channel-separated test");
    println!("seek_ms={seek_ms}"); println!("loop_mode=browser_worklet");
    let input_meta=fs::metadata("/in/input.vgm").map_err(|e|format!("failed to read input metadata: {e}"))?;
    println!("input_bytes={}",input_meta.len());
    let bytes=load_vgm_input()?; println!("vgm_decompressed_bytes={}",bytes.len());
    if bytes.len()<0x40||&bytes[0..4]!=b"Vgm "{return Err("not a VGM file".into());}
    let version=u32le(&bytes,0x08);
    let data_off=if version>=0x0001_0050{let r=u32le(&bytes,0x34);if r==0{0x40}else{0x34+r as usize}}else{0x40};
    let eof_rel=u32le(&bytes,0x04); let eof=if eof_rel==0{bytes.len()}else{(0x04+eof_rel as usize).min(bytes.len())};
    let clock_raw=u32le(&bytes,0x2c); let clock=if clock_raw&0x3fff_ffff!=0{clock_raw&0x3fff_ffff}else{FALLBACK_YM_CLOCK};
    println!("vgm_version=0x{version:08X}");println!("vgm_data_offset=0x{data_off:X}");println!("ym2612_clock={clock}");
    let probe=ffi::create_chip(ffi::ChipType::Ym3438,clock); let rate=probe.sample_rate() as u32; let chip_channels=probe.channels() as usize;
    println!("ym3438_channels={chip_channels}");println!("ym3438_native_rate={rate}");
    if data_off>=eof{return Err("invalid VGM data offset".into());}
    let target_frames=((seek_ms as u128*rate as u128)/1000) as u64; println!("seek_target_native_frames={target_frames}");
    let loop_rel=u32le(&bytes,0x1c); let loop_pos=if loop_rel==0{None}else{Some(0x1c+loop_rel as usize)}; let loop_samples_header=u32le(&bytes,0x20);
    println!("vgm_loop_position={loop_pos:?}");println!("vgm_loop_samples={loop_samples_header}");

    let parts=[RenderPart::Fm(0),RenderPart::Fm(1),RenderPart::Fm(2),RenderPart::Fm(3),RenderPart::Fm(4),RenderPart::Fm(5),RenderPart::Dac];
    let labels=["FM1","FM2","FM3","FM4","FM5","FM6","DAC"];
    let mut streams:Vec<Vec<i16>>=Vec::with_capacity(7); let mut timeline=0u64; let mut peak=0i32; let mut totals=[0u64;4];
    println!("channel_separation=7 stereo streams (FM1-FM6 + DAC)");
    for (part,label) in parts.iter().zip(labels.iter()) {
        let (pcm,t,pk,cmds,yms,dacs,blocks)=render_part(&bytes,data_off,eof,clock,rate,target_frames,*part,label)?;
        if timeline==0{timeline=t;} else if timeline!=t{return Err("channel stream timeline mismatch".into());}
        peak=peak.max(pk); totals[0]+=cmds; totals[1]+=yms; totals[2]+=dacs; totals[3]=totals[3].max(blocks); streams.push(pcm);
    }
    let frames=streams[0].len()/2; if frames==0||peak==0{return Err("VGM parsing completed but PCM is silent".into());}
    for s in &streams{if s.len()!=streams[0].len(){return Err("channel PCM length mismatch".into());}}
    println!("commands_total_across_parts={}",totals[0]);println!("ym_register_writes_total_across_parts={}",totals[1]);println!("dac_writes_total_across_parts={}",totals[2]);println!("data_blocks={}",totals[3]);
    let duration=timeline as f64/VGM_RATE as f64; let loop_start=loop_pos.and_then(|lp|{find_loop_samples(&bytes,data_off,eof,lp)}).map(|v|v as f64/VGM_RATE as f64);
    let loop_end=if loop_start.is_some(){Some(duration)}else{None};
    println!("duration_seconds={duration:.6}");println!("timeline_duration_seconds={duration:.6}");
    println!("loop_start_seconds={}",loop_start.map(|v|format!("{v:.6}")).unwrap_or_else(||"none".into()));
    println!("loop_end_seconds={}",loop_end.map(|v|format!("{v:.6}")).unwrap_or_else(||"none".into()));println!("loop_prepared={}",loop_start.is_some());
    println!("generated_frames={frames}");println!("seek_effective_seconds={:.6}",target_frames as f64/rate as f64);println!("peak_raw={peak}");println!("output_gain={OUTPUT_GAIN}");
    println!("pcm_channels_total=14"); println!("pcm_layout=FM1_L,FM1_R,FM2_L,FM2_R,FM3_L,FM3_R,FM4_L,FM4_R,FM5_L,FM5_R,FM6_L,FM6_R,DAC_L,DAC_R");
    let mut pcm_bytes=Vec::with_capacity(frames*14*2);
    for i in 0..frames{for s in &streams{pcm_bytes.extend_from_slice(&s[i*2].to_le_bytes());pcm_bytes.extend_from_slice(&s[i*2+1].to_le_bytes());}}
    const OUTPUT_PATH:&str="/out/ym3438_output.pcm"; fs::write(OUTPUT_PATH,&pcm_bytes).map_err(|e|format!("failed to write {OUTPUT_PATH}: {e}"))?;
    println!("pcm_path={OUTPUT_PATH}");println!("pcm_bytes={}",pcm_bytes.len());println!("pcm_channels=14");println!("pcm_source_channels=2");println!("pcm_sample_rate={rate}");
    Ok(())
}

fn find_loop_samples(bytes:&[u8],data_off:usize,eof:usize,loop_pos:usize)->Option<u64>{
    let mut pos=data_off;let mut t=0u64;
    while pos<eof{if pos==loop_pos{return Some(t);}let c=bytes[pos];match c{0x61=>{if pos+3>eof{return None;}t+=u16::from_le_bytes([bytes[pos+1],bytes[pos+2]]) as u64;pos+=3;},0x62=>{t+=735;pos+=1;},0x63=>{t+=882;pos+=1;},0x70..=0x7f=>{t+=(c&0xf) as u64+1;pos+=1;},0x80..=0x8f=>{t+=(c&0xf) as u64;pos+=1;},0x66=>return None,_=>{pos+=command_size(bytes,pos,eof)?;}}}None}
