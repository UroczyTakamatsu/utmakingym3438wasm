class PcmPlayerProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.current = null;
    this.offset = 0;       // interleaved-sample offset in current
    this.sourcePos = 0;    // source-frame position relative to current offset
    this.sourceRate = sampleRate;
    this.ended = false;
    this.endNotified = false;

    this.port.onmessage = (event) => {
      const m = event.data;
      if (m.type === 'init') {
        this.queue.length = 0;
        this.current = null;
        this.offset = 0;
        this.sourcePos = 0;
        this.sourceRate = m.sampleRate || sampleRate;
        this.ended = false;
        this.endNotified = false;
      } else if (m.type === 'chunk') {
        this.queue.push(new Int16Array(m.buffer));
      } else if (m.type === 'end') {
        this.ended = true;
      } else if (m.type === 'stop') {
        this.queue.length = 0;
        this.current = null;
        this.offset = 0;
        this.sourcePos = 0;
        this.ended = false;
        this.endNotified = false;
      }
    };
  }

  // Return one stereo frame at a frame index relative to the current read offset.
  getFrame(frameIndex) {
    let sampleIndex = frameIndex * 2;

    if (this.current !== null) {
      const available = this.current.length - this.offset;
      if (sampleIndex + 1 < available) {
        const p = this.offset + sampleIndex;
        return [this.current[p], this.current[p + 1]];
      }
      sampleIndex -= available;
    }

    for (const q of this.queue) {
      if (sampleIndex + 1 < q.length) {
        return [q[sampleIndex], q[sampleIndex + 1]];
      }
      sampleIndex -= q.length;
    }
    return null;
  }

  discardFrames(frames) {
    let samples = frames * 2;
    while (samples > 0) {
      if (this.current === null) {
        if (this.queue.length === 0) return;
        this.current = this.queue.shift();
        this.offset = 0;
      }

      const available = this.current.length - this.offset;
      const take = Math.min(samples, available);
      this.offset += take;
      samples -= take;

      if (this.offset >= this.current.length) {
        this.current = null;
        this.offset = 0;
      }
    }
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    const left = output[0];
    const right = output[1] || output[0];
    const ratio = this.sourceRate / sampleRate;

    for (let i = 0; i < left.length; i++) {
      const base = Math.floor(this.sourcePos);
      const frac = this.sourcePos - base;
      const a = this.getFrame(base);
      const b = this.getFrame(base + 1);

      if (a === null || b === null) {
        left[i] = 0;
        right[i] = 0;

        if (this.ended && !this.endNotified) {
          this.endNotified = true;
          this.port.postMessage({ type: 'ended' });
        }
        continue;
      }

      left[i] = (a[0] + (b[0] - a[0]) * frac) / 32768;
      right[i] = (a[1] + (b[1] - a[1]) * frac) / 32768;

      this.sourcePos += ratio;

      // Remove only the whole source frames that are now behind the cursor.
      const discard = Math.floor(this.sourcePos);
      if (discard > 0) {
        this.discardFrames(discard);
        this.sourcePos -= discard;
      }
    }

    return true;
  }
}

registerProcessor('pcm-player', PcmPlayerProcessor);
