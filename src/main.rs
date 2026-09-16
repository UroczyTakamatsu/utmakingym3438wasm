use ymfm_sys::ffi;

fn main() {
    let clock: u32 = 7_670_454;
    let mut chip = ffi::create_chip(ffi::ChipType::Ym3438, clock);

    let channels = chip.channels();
    let rate = chip.sample_rate();

    chip.pin_mut().reset();

    // YM3438/OPN2 channel 0. Enable output to both left and right.
    chip.pin_mut().write(0, 0xB4);
    chip.pin_mut().write(1, 0xC0);

    // Algorithm 7: all four operators feed the output.
    chip.pin_mut().write(0, 0xB0);
    chip.pin_mut().write(1, 0x07);

    // Give all four operators an audible level, fast attack, and release.
    // OPN2 operator register offsets for channel 0 are 0, 4, 8, 12.
    for op in [0x00u8, 0x04, 0x08, 0x0C] {
        chip.pin_mut().write(0, 0x40 + op);
        chip.pin_mut().write(1, 0x00); // TL = 0 dB attenuation

        chip.pin_mut().write(0, 0x50 + op);
        chip.pin_mut().write(1, 0x1F); // AR = maximum

        chip.pin_mut().write(0, 0x60 + op);
        chip.pin_mut().write(1, 0x00); // DR = 0

        chip.pin_mut().write(0, 0x70 + op);
        chip.pin_mut().write(1, 0x00); // SR = 0

        chip.pin_mut().write(0, 0x80 + op);
        chip.pin_mut().write(1, 0x0F); // RR = maximum

        chip.pin_mut().write(0, 0x30 + op);
        chip.pin_mut().write(1, 0x01); // MUL = 1
    }

    // F-number/block for a clearly audible test tone.
    chip.pin_mut().write(0, 0xA0);
    chip.pin_mut().write(1, 0x98);
    chip.pin_mut().write(0, 0xA4);
    chip.pin_mut().write(1, 0x22);

    // Key ON, channel 0, all four operators.
    chip.pin_mut().write(0, 0x28);
    chip.pin_mut().write(1, 0xF0);

    let frames = 4096usize;
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
