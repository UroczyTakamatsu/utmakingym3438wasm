use ymfm_sys::ffi::{self, ChipType};

fn main() {
    // YM3438 is clocked at the standard Mega Drive/Genesis FM clock used by
    // many YM2612-compatible VGM files. The exact VGM clock will be handled
    // later by the VGM player; this first stage only proves that the actual
    // YM3438 implementation is linked into WASM and can generate samples.
    let clock = 7_670_454u32;
    let mut chip = ffi::create_chip(ChipType::Ym3438, clock);

    let channels = chip.channels() as usize;
    let sample_rate = chip.sample_rate();
    assert!(channels > 0);
    assert!(sample_rate > 0);

    // Generate a small block. This exercises the C++ ymfm YM3438 core.
    let mut samples = vec![0i32; channels * 256];
    chip.pin_mut().generate(&mut samples);

    let nonzero = samples.iter().filter(|&&v| v != 0).count();
    println!("YM3438 OK: channels={channels}, sample_rate={sample_rate}, generated_samples={}, nonzero={nonzero}", samples.len());
}
