class PcmPlayerProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.current = null;
    this.offset = 0;
    this.sourceRate = sampleRate;
    this.position = 0;
    this.ended = false;
    this.port.onmessage = (event) => {
      const m = event.data;
      if (m.type === 'init') {
        this.sourceRate = m.sampleRate || sampleRate;
        this.position = 0;
        this.ended = false;
      } else if (m.type === 'chunk') {
        this.queue.push(new Int16Array(m.buffer));
      } else if (m.type === 'end') {
        this.ended = true;
      } else if (m.type === 'stop') {
        this.queue.length = 0;
        this.current = null;
        this.offset = 0;
        this.position = 0;
        this.ended = false;
      }
    };
  }

  getSample(index) {
    while (this.current === null || index >= this.current.length) {
      if (this.current !== null) index -= this.current.length;
      if (this.queue.length === 0) return null;
      this.current = this.queue.shift();
      this.offset = 0;
    }
    return this.current[index];
  }

  consumeSample() {
    if (!this.current) return null;
    const v = this.current[this.offset++];
    if (this.offset >= this.current.length) {
      this.current = null;
      this.offset = 0;
    }
    return v;
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    const left = output[0];
    const right = output[1] || output[0];
    const channels = 2;
    const ratio = this.sourceRate / sampleRate;

    for (let i = 0; i < left.length; i++) {
      const base = Math.floor(this.position);
      const frac = this.position - base;
      const aL = this.peekInterleaved(base * channels);
      const bL = this.peekInterleaved((base + 1) * channels);
      if (aL === null || bL === null) {
        left[i] = 0;
        right[i] = 0;
        if (this.ended && this.queue.length === 0 && this.current === null) {
          this.port.postMessage({ type: 'ended' });
        }
        continue;
      }
      left[i] = ((aL[0] + (bL[0] - aL[0]) * frac) / 32768);
      const rv = ((aL[1] + (bL[1] - aL[1]) * frac) / 32768);
      right[i] = rv;
      this.position += ratio;
      this.discardBefore(Math.floor(this.position));
    }
    return true;
  }

  // This worklet keeps a small resampling cursor over interleaved stereo PCM.
  // The queue is chunked, but the cursor must be able to look ahead one frame.
  peekInterleaved(sampleIndex) {
    let idx = sampleIndex;
    if (this.current !== null) {
      if (idx < this.current.length - this.offset) {
        return [this.current[this.offset + idx], this.current[this.offset + idx + 1]];
      }
      idx -= (this.current.length - this.offset);
    }
    for (const q of this.queue) {
      if (idx + 1 < q.length) return [q[idx], q[idx + 1]];
      idx -= q.length;
    }
    return null;
  }

  discardBefore(frames) {
    let remaining = frames * 2;
    while (remaining > 0) {
      if (!this.current) {
        if (this.queue.length === 0) return;
        this.current = this.queue.shift();
        this.offset = 0;
      }
      const available = this.current.length - this.offset;
      const take = Math.min(remaining, available);
      this.offset += take;
      remaining -= take;
      if (this.offset >= this.current.length) {
        this.current = null;
        this.offset = 0;
      }
    }
  }
}

registerProcessor('pcm-player', PcmPlayerProcessor);
