YM3438 VGM/VGZ seamless loop behavior fix.

Changed files:
- src/main.rs
- web/index.html
- web/worklet.js

Behavior:
- VGM loop information is always rendered/prepared when a loop exists, regardless of UI loop ON/OFF.
- Loop OFF stops at the first loop end and does not play the second loop pass.
- Loop ON can be enabled before or during the loop without rerunning WASM.
- While looped, the seek bar/progress reports the corresponding first-pass logical position, never accumulating loop repetitions.
- Turning loop OFF during the loop maps playback back to the corresponding first-pass position and then stops at the loop end.
- Seek duration excludes the repeated loop pass.
