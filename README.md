# YM3438 VGM/VGZ renderer — verification v2

This version is a diagnostic/offline verification stage.

## Test files included

- `testdata/01 - Opening Theme.vgz`
- `testdata/03 - Emerald Hill Zone.vgz`

The files are included directly in the repository so the workflow does **not** download test assets from an external URL.

## Important fixes from v1

The first renderer stopped at the initial `0x67` data block because that command was not parsed correctly. These Mega Drive VGM files use a YM2612 DAC data block at the beginning.

v2 adds:

- VGM `0x67` data-block parsing.
- YM2612 DAC data bank type `0x00`.
- VGM `0x80..0x8F` YM2612 DAC streaming commands.
- VGM `0xE0` DAC data-bank seek.
- Correct `0x80..0x8F` wait timing: low nibble is `0..15` samples.
- Correct command-length handling for common VGM commands.
- Detailed diagnostic counters.
- Refusal to write a header-only 44-byte WAV.
- Both supplied test VGZ files rendered by the same GitHub Actions job.

The `0x80..0x8F` interpretation is based on the VGM format behavior documented by VGMRips: these commands resolve to YM2612 register `0x2A` DAC writes plus a wait, while `0xE0` selects a position in the PCM data bank. citeturn0search0turn0search5

## Expected diagnostic result

For each track, Actions should print values such as:

- decompressed VGM size
- VGM version
- YM2612 clock
- YM3438 native sample rate
- data block count
- DAC stream writes
- YM2612/YM3438 register writes
- VGM wait samples
- generated PCM frames
- peak sample
- WAV file size

The most important values are:

`Generated PCM frames`

and

`WAV written: ... (N bytes)`

If `Generated PCM frames` is non-zero, the previous 44-byte WAV problem has been resolved.

## WASM

The WASM artifact is still a `wasm32-wasip1` executable. It is **not yet** the final browser-callable module.

The next stage after successful WAV verification is to expose a browser-callable interface and then connect it to a Worker/AudioWorklet architecture.
