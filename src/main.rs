use ymfm_sys::ffi;

fn main() {
    let clock: u32 = 7_670_454;
    let mut chip = ffi::create_chip(ffi::ChipType::Ym3438, clock);

    let channels = chip.channels();
    let rate = chip.sample_rate();

    // Reset and program a simple YM3438 tone.
    chip.pin_mut().reset();

    // Channel 0: algorithm 7, both operators audible.
    chip.pin_mut().write(0, 0xB0);
    chip.pin_mut().write(1, 0x07);

    // Operator 1/2 total levels.
    chip.pin_mut().write(0, 0x40);
    chip.pin_mut().write(1, 0x00);
    chip.pin_mut().write(0, 0x44);
    chip.pin_mut().write(1, 0x00);

    // F-number.
    chip.pin_mut().write(0, 0xA0);
    chip.pin_mut().write(1, 0x98);
    chip.pin_mut().write(0, 0xA4);
    chip.pin_mut().write(1, 0x22);

    // Key ON, channel 0, all four operators.
    chip.pin_mut().write(0, 0x28);
    chip.pin_mut().write(1, 0xF0);

    let frames = 2048usize;
    let mut pcm = vec![0i32; frames * channels as usize];
    chip.pin_mut().generate(&mut pcm);

    let mut peak = 0i32;
    let mut nonzero = 0usize;
    for &v in &pcm {
        peak = peak.max(v.abs());
        if v != 0 {
            nonzero += 1;
        }
    }

    println!("YM3438 browser execution test");
    println!("clock={clock}");
    println!("channels={channels}");
    println!("native_sample_rate={rate}");
    println!("generated_frames={frames}");
    println!("nonzero_samples={nonzero}");
    println!("peak={peak}");

    if nonzero == 0 || peak == 0 {
        std::process::exit(2);
    }
}
