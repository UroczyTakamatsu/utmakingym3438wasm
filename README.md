# YM3438 Browser Execution Test (rebuilt)

This test takes a different route from the previous attempts.

The current `ymfm-sys` crate is a Rust library, so asking Cargo to copy
`ymfm-sys.wasm` was incorrect: a library dependency naturally produced
`libymfm_sys.rlib`.

Instead, this project builds a small WASI executable that links against
`ymfm-sys`, creates a YM3438, writes registers, calls `generate()`, and
prints PCM-generation diagnostics.

The browser then runs that WASI executable using the Wasmer JavaScript SDK.

This deliberately does NOT use AudioWorklet yet. The goal is to prove:

browser -> Wasmer/WASI -> our Rust executable -> ymfm-sys -> YM3438 -> PCM

After this succeeds, the next step is to expose a persistent YM3438 instance
for streaming instead of launching a short-lived test executable.
