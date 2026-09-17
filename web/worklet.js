class PcmPlayerProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks = [];
    this.totalFrames = 0;
    this.ready = false;
    this.sourcePos = 0;
    this.sourceRate = sampleRate;
    this.ended = false;
    this.endNotified = false;
    this.paused = false;
    this.loopEnabled = false;
    this.loopStartFrame = 0;
    this.loopEndFrame = 0;
    this.timelineStartSeconds = 0;
    this.timelinePositionSeconds = 0;
    this.reportCounter = 0;

    this.port.onmessage = (event) => {
      const m = event.data;
      if (m.type === 'init') {
        this.chunks = [];
        this.totalFrames = 0;
        this.ready = false;
        this.sourceRate = m.sampleRate || sampleRate;
        this.sourcePos = 0;
        this.ended = false;
        this.endNotified = false;
        this.paused = false;
        this.loopEnabled = !!m.loopEnabled;
        this.timelineStartSeconds = Number(m.startSeconds) || 0;
        this.timelinePositionSeconds = this.timelineStartSeconds;

        const ls = Number(m.loopStartSeconds);
        const le = Number(m.loopEndSeconds);
        const start = this.timelineStartSeconds;
        this.loopStartFrame = Number.isFinite(ls) ? Math.max(0, (ls - start) * this.sourceRate) : 0;
        this.loopEndFrame = Number.isFinite(le) ? Math.max(0, (le - start) * this.sourceRate) : 0;
        this.reportCounter = 0;
      } else if (m.type === 'chunk') {
        const data = new Int16Array(m.buffer);
        this.chunks.push({ data, startFrame: this.totalFrames });
        this.totalFrames += Math.floor(data.length / 2);
      } else if (m.type === 'end') {
        this.ended = true;
        this.ready = true;
      } else if (m.type === 'pause') {
        this.paused = true;
      } else if (m.type === 'resume') {
        this.paused = false;
      } else if (m.type === 'setLoop') {
        this.loopEnabled = !!m.enabled;
        this.endNotified = false;
      } else if (m.type === 'stop') {
        this.chunks = [];
        this.totalFrames = 0;
        this.ready = false;
        this.sourcePos = 0;
        this.ended = false;
        this.endNotified = false;
        this.paused = false;
        this.loopEnabled = false;
        this.loopStartFrame = 0;
        this.loopEndFrame = 0;
        this.timelineStartSeconds = 0;
        this.timelinePositionSeconds = 0;
        this.reportCounter = 0;
      }
    };
  }

  getFrame(frameIndex) {
    if (frameIndex < 0 || frameIndex >= this.totalFrames) return null;
    for (const chunk of this.chunks) {
      if (frameIndex >= chunk.startFrame) {
        const local = frameIndex - chunk.startFrame;
        const p = local * 2;
        if (p + 1 < chunk.data.length) return [chunk.data[p], chunk.data[p + 1]];
      } else break;
    }
    return null;
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    const left = output[0];
    const right = output[1] || output[0];

    if (this.paused || !this.ready) {
      left.fill(0);
      if (output[1]) right.fill(0);
      return true;
    }

    const ratio = this.sourceRate / sampleRate;
    const hasLoop = this.loopEndFrame > this.loopStartFrame;

    for (let i = 0; i < left.length; i++) {
      if (hasLoop && this.loopEnabled && this.sourcePos >= this.loopEndFrame) {
        const len = this.loopEndFrame - this.loopStartFrame;
        this.sourcePos = this.loopStartFrame + ((this.sourcePos - this.loopEndFrame) % len);
        this.endNotified = false;
        this.port.postMessage({ type: 'loop' });
      }

      if (hasLoop && !this.loopEnabled && this.sourcePos >= this.loopEndFrame) {
        if (!this.endNotified) {
          this.endNotified = true;
          this.port.postMessage({ type: 'ended' });
        }
        left[i] = 0;
        right[i] = 0;
        continue;
      }

      if (this.sourcePos >= this.totalFrames - 1) {
        if (!this.endNotified) {
          this.endNotified = true;
          this.port.postMessage({ type: 'ended' });
        }
        left[i] = 0;
        right[i] = 0;
        continue;
      }

      const base = Math.floor(this.sourcePos);
      const frac = this.sourcePos - base;
      const a = this.getFrame(base);
      const b = this.getFrame(base + 1);
      if (!a || !b) {
        left[i] = 0;
        right[i] = 0;
        continue;
      }

      left[i] = (a[0] + (b[0] - a[0]) * frac) / 32768;
      right[i] = (a[1] + (b[1] - a[1]) * frac) / 32768;
      this.sourcePos += ratio;

      const timelineFrame = this.sourcePos;
      this.timelinePositionSeconds = this.timelineStartSeconds + timelineFrame / this.sourceRate;
      this.reportCounter++;
      if (this.reportCounter >= 12) {
        this.reportCounter = 0;
        let seconds = this.timelinePositionSeconds;
        if (hasLoop && this.loopEnabled && seconds >= this.timelineStartSeconds + this.loopEndFrame / this.sourceRate) {
          const loopLen = (this.loopEndFrame - this.loopStartFrame) / this.sourceRate;
          seconds = this.timelineStartSeconds + this.loopStartFrame / this.sourceRate +
            ((seconds - (this.timelineStartSeconds + this.loopEndFrame / this.sourceRate)) % loopLen);
        }
        this.port.postMessage({ type: 'progress', seconds });
      }
    }

    return true;
  }
}

registerProcessor('pcm-player', PcmPlayerProcessor);
