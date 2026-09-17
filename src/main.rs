use flate2::read::GzDecoder;
use std::env;
use std::fs;
use std::io::Read;
use ymfm_sys::ffi;

const VGM_RATE: u64 = 44_100;
const FALLBACK_YM_CLOCK: u32 = 7_670_454;
const OUTPUT_GAIN: i32 = 128;

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
fn read_u32(b:&[u8],p:usize)->u32{u32::from_le_bytes([b[p],b[p+1],b[p+2],b[p+3]])}
fn read_u16(b:&[u8],p:usize)->u16{u16::from_le_bytes([b[p],b[p+1]])}
fn decode_vgm(input:&[u8])->Result<Vec<u8>,String>{
    if input.len()>=2 && input[0]==0x1f && input[1]==0x8b {
        let mut d=GzDecoder::new(input); let mut out=Vec::new();
        d.read_to_end(&mut out).map_err(|e|format!("VGZ展開失敗: {e}"))?; Ok(out)
    } else { Ok(input.to_vec()) }
}
fn render_wait(chip:&mut cxx::UniquePtr<ffi::Chip>,out:&mut Vec<i16>,channels:usize,native_rate:u32,wait:u64,timing:&mut TimingAccumulator,peak:&mut i32,skip_frames:&mut u64){
    if wait==0{return}
    let n=timing.frames_for_wait(wait,native_rate); if n==0{return}
    let mut buf=vec![0i32;n*channels]; chip.pin_mut().generate(&mut buf);
    for &x in &buf{*peak=(*peak).max(x.abs());}
    let skip=(*skip_frames).min(n as u64) as usize; *skip_frames-=skip as u64;
    for &x in &buf[skip*channels..]{out.push(((x>>8)*OUTPUT_GAIN).clamp(i16::MIN as i32,i16::MAX as i32) as i16);}
}
fn run()->Result<(),String>{
    let args:Vec<String>=env::args().skip(1).collect();
    let seek_ms:u64=args.iter().find_map(|a|a.strip_prefix("--seek-ms=").and_then(|v|v.parse().ok())).unwrap_or(0);
    println!("YM3438 browser VGM/VGZ -> PCM diagnostic test"); println!("seek_ms={seek_ms}");
    let input=fs::read("/in/input.vgm").map_err(|e|format!("入力読み込み失敗: {e}"))?;
    println!("input_bytes={}",input.len()); let bytes=decode_vgm(&input)?; println!("vgm_decompressed_bytes={}",bytes.len());
    if bytes.len()<0x40 || &bytes[0..4]!=b"Vgm "{return Err("VGMヘッダが見つかりません".into());}
    let version=read_u32(&bytes,0x08);
    let data_offset=if version>=0x150{let r=read_u32(&bytes,0x34);if r==0{0x40}else{0x34+r as usize}}else{0x40};
    let total_samples=read_u32(&bytes,0x18) as u64;
    let loop_rel=read_u32(&bytes,0x1c); let loop_pos=if loop_rel==0{None}else{Some(0x1cusize+loop_rel as usize)};
    println!("vgm_version=0x{version:08X}"); println!("vgm_data_offset=0x{data_offset:X}"); println!("vgm_total_samples={total_samples}");
    println!("vgm_loop_position={}",loop_pos.map(|p|format!("0x{p:X}")).unwrap_or_else(||"none".into()));
    let clock={let c=read_u32(&bytes,0x2c);if c==0{FALLBACK_YM_CLOCK}else{c}};
    let mut chip=ffi::create_chip(ffi::ChipType::Ym3438,clock); if chip.is_null(){return Err("YM3438チップ生成失敗".into());}
    let channels=chip.channels() as usize; let native_rate=chip.sample_rate() as u32; chip.pin_mut().reset();
    println!("ym2612_clock={clock}"); println!("ym3438_channels={channels}"); println!("ym3438_native_rate={native_rate}");
    let target_frames=((seek_ms as u128*native_rate as u128)/1000) as u64; let mut skip_frames=target_frames;
    let mut pos=data_offset; let eof=bytes.len(); let mut timeline_samples=0u64;
    let mut loop_start_samples=None::<u64>; let mut loop_end_samples=None::<u64>;
    let mut loop_start_output_frame=None::<u64>; let mut loop_end_output_frame=None::<u64>;
    let mut timing=TimingAccumulator::new(); let mut out=Vec::<i16>::new(); let mut commands=0u64; let mut register_writes=0u64; let mut ym_register_writes=0u64; let mut dac_writes=0u64; let mut data_blocks=0u64; let mut wait_samples=0u64; let mut peak=0i32; let mut dac_data=Vec::new(); let mut dac_pos=0usize;
    while pos<eof{
        if Some(pos)==loop_pos && loop_start_samples.is_none(){
            loop_start_samples=Some(timeline_samples);
            if timeline_samples.saturating_mul(native_rate as u64) >= target_frames.saturating_mul(VGM_RATE){loop_start_output_frame=Some((out.len()/channels) as u64);}
        }
        let cmd=bytes[pos]; commands+=1;
        match cmd{
            0x50=>{if pos+2>eof{return Err("0x50末尾不正".into())}pos+=2;register_writes+=1;}
            0x52|0x53=>{if pos+3>eof{return Err(format!("0x{cmd:02X}末尾不正"))}chip.pin_mut().write((cmd-0x52) as u8,bytes[pos+1],bytes[pos+2]);pos+=3;register_writes+=1;ym_register_writes+=1;}
            0x61=>{if pos+3>eof{return Err("0x61末尾不正".into())}let n=read_u16(&bytes,pos+1) as u64;wait_samples+=n;render_wait(&mut chip,&mut out,channels,native_rate,n,&mut timing,&mut peak,&mut skip_frames);timeline_samples+=n;pos+=3;}
            0x62=>{let n=735;wait_samples+=n;render_wait(&mut chip,&mut out,channels,native_rate,n,&mut timing,&mut peak,&mut skip_frames);timeline_samples+=n;pos+=1;}
            0x63=>{let n=882;wait_samples+=n;render_wait(&mut chip,&mut out,channels,native_rate,n,&mut timing,&mut peak,&mut skip_frames);timeline_samples+=n;pos+=1;}
            0x67=>{if pos+7>eof||bytes[pos+1]!=0x66{return Err(format!("0x67不正: 0x{pos:X}"))}let typ=bytes[pos+2];let len=read_u32(&bytes,pos+3) as usize;let start=pos+7;let end=start.checked_add(len).ok_or("0x67長さオーバーフロー")?;if end>eof{return Err("0x67が末尾越え".into())}if typ==0{dac_data=bytes[start..end].to_vec();dac_pos=0;}data_blocks+=1;pos=end;}
            0x70..=0x7f=>{let n=(cmd&0x0f) as u64+1;wait_samples+=n;render_wait(&mut chip,&mut out,channels,native_rate,n,&mut timing,&mut peak,&mut skip_frames);timeline_samples+=n;pos+=1;}
            0x80..=0x8f=>{let n=(cmd&0x0f) as u64;if dac_pos<dac_data.len(){chip.pin_mut().write(0,0x2a,dac_data[dac_pos]);dac_pos+=1;dac_writes+=1;}if n>0{wait_samples+=n;render_wait(&mut chip,&mut out,channels,native_rate,n,&mut timing,&mut peak,&mut skip_frames);timeline_samples+=n;}pos+=1;}
            0xe0=>{if pos+5>eof{return Err("0xE0末尾不正".into())}dac_pos=read_u32(&bytes,pos+1) as usize;pos+=5;}
            0x66=>{loop_end_samples=if loop_pos.is_some(){Some(timeline_samples)}else{None};loop_end_output_frame=if loop_pos.is_some(){Some((out.len()/channels) as u64)}else{None};pos+=1;break;}
            _=>return Err(format!("未対応VGMコマンド 0x{cmd:02X} at 0x{pos:X}")),
        }
        if let Some(ls)=loop_start_samples{if loop_start_output_frame.is_none() && ls.saturating_mul(native_rate as u64)<target_frames.saturating_mul(VGM_RATE){loop_start_output_frame=Some(0);}}
    }
    if loop_pos.is_some(){loop_end_samples.get_or_insert(timeline_samples);loop_end_output_frame.get_or_insert((out.len()/channels) as u64);}
    let frames=(out.len()/channels) as u64; let duration=timeline_samples as f64/VGM_RATE as f64;
    println!("commands={commands}");println!("register_writes={register_writes}");println!("ym_register_writes={ym_register_writes}");println!("dac_writes={dac_writes}");println!("data_blocks={data_blocks}");println!("wait_samples={wait_samples}");println!("duration_seconds={duration:.6}");println!("timeline_duration_seconds={duration:.6}");println!("generated_frames={frames}");println!("output_total_frames={frames}");println!("peak_raw={peak}");println!("output_gain={OUTPUT_GAIN}");println!("timing_remainder_1_44100={}",timing.remainder);println!("skipped_native_frames={}",target_frames.saturating_sub(skip_frames));println!("seek_effective_seconds={:.6}",target_frames as f64/native_rate as f64);
    if let Some(v)=loop_start_samples{println!("loop_start_seconds={:.6}",v as f64/VGM_RATE as f64)}else{println!("loop_start_seconds=none")}
    if let Some(v)=loop_end_samples{println!("loop_end_seconds={:.6}",v as f64/VGM_RATE as f64)}else{println!("loop_end_seconds=none")}
    if let Some(v)=loop_start_output_frame{println!("loop_start_output_frame={v}")}else{println!("loop_start_output_frame=none")}
    if let Some(v)=loop_end_output_frame{println!("loop_end_output_frame={v}")}else{println!("loop_end_output_frame=none")}
    if frames==0{return Err("PCMが生成されませんでした".into())}
    let mut pcm=Vec::with_capacity(out.len()*2);for s in out{pcm.extend_from_slice(&s.to_le_bytes());}
    fs::create_dir_all("/out").map_err(|e|format!("/out作成失敗: {e}"))?;fs::write("/out/ym3438_output.pcm",&pcm).map_err(|e|format!("PCM書き込み失敗: {e}"))?;
    println!("pcm_path=/out/ym3438_output.pcm");println!("pcm_bytes={}",pcm.len());println!("pcm_channels={channels}");println!("pcm_sample_rate={native_rate}");Ok(())
}
fn main(){if let Err(e)=run(){eprintln!("ERROR: {e}");std::process::exit(1);}}
