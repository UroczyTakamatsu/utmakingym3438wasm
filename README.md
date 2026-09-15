# YM3438 WASM minimum build

This is the first build-only stage for the YM3438 VGM player.

It intentionally does **not** contain the browser player yet. Its only goal is to prove that:

1. GitHub Actions fetches `h1romas4/ymfm-sys` with `--recurse-submodules`.
2. The upstream `ymfm` source exists in `components/ymfm`.
3. `ChipType::Ym3438` links successfully through the C++ shim.
4. `wasm32-wasip1` can be produced.
5. The resulting WASM generates samples through the real YM3438 core.

## How to use on Android

1. Create a new GitHub repository.
2. Upload this project's files.
3. Open **Actions**.
4. Select **Build YM3438 WASM**.
5. Run the workflow with **Run workflow**.
6. When it finishes, open the run's **Artifacts** section and download `ym3438-wasm`.

The next stage will replace the smoke-test program with a VGM/VGZ streaming API and browser audio output.

## Why `--recurse-submodules` matters

The `ymfm-sys` repository keeps the upstream ymfm implementation in its `components/ymfm` submodule. A plain ZIP of the repository can contain an empty submodule directory; the recursive clone fetches the actual source.
