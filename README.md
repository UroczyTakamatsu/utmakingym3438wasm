# YM3438 VGM/VGZ renderer

This project extends the successfully building ymfm-sys + YM3438 WASM setup.

## What it does

- Builds ymfm-sys with its upstream ymfm submodule.
- Uses YM3438.
- Accepts VGM or VGZ input.
- Parses YM2612-compatible VGM commands `0x52` and `0x53` and sends them to YM3438.
- Handles waits `0x61`, `0x62`, `0x63`, and `0x70..0x7F`.
- Handles VGM end and one loop pass.
- Decompresses VGZ with gzip.
- Produces a WASM executable and can produce a native WAV verification file.

## Important

This is the **offline verification stage**. The WASM is currently a WASI executable, not yet the final browser C-ABI module.

The next stage will expose a browser-callable C ABI and connect it to JavaScript/AudioWorklet.

## GitHub Actions

The workflow intentionally starts from the previously successful setup:

1. `actions/checkout@v5`
2. recursive clone of `ymfm-sys`
3. `std::abs` WASI compatibility patch
4. WASI SDK 34
5. `wasm32-wasip1` build

The VGM renderer is then compiled on top of that.

## VGM/VGZ test files

For the next verification step, use:

- `01 - Opening Theme.vgz` — short, no loop
- `03 - Emerald Hill Zone.vgz` — long, looped

The current workflow only attempts the first asset from a release URL. This is deliberate: do not depend on an unverified external asset location for the core build.

The safest next step is to upload the test VGZ files directly to the repository and add them to a later verification job.
