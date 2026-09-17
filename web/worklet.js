class PcmPlayerProcessor extends AudioWorkletProcessor {
  constructor(){
    super();
    this.chunks=[]; this.pcm=null; this.totalFrames=0; this.ready=false;
    this.sourcePos=0; this.sourceRate=sampleRate; this.startSeconds=0;
    this.loopEnabled=false; this.hasLoop=false;
    this.loopStartFrame=null; this.loopEndFrame=null;
    this.paused=false; this.endNotified=false; this.reportCounter=0;
    this.port.onmessage=e=>{
      const m=e.data||{};
      if(m.type==='init'){
        this.chunks=[]; this.pcm=null; this.totalFrames=Number(m.totalFrames)||0;
        this.ready=false; this.sourceRate=Number(m.sampleRate)||sampleRate;
        this.sourcePos=0; this.startSeconds=Number(m.startSeconds)||0;
        this.loopEnabled=!!m.loopEnabled; this.hasLoop=!!m.hasLoop;
        this.loopStartFrame=Number.isFinite(Number(m.loopStartFrame))?Number(m.loopStartFrame):null;
        this.loopEndFrame=Number.isFinite(Number(m.loopEndFrame))?Number(m.loopEndFrame):null;
        this.paused=false; this.endNotified=false; this.reportCounter=0;
      }else if(m.type==='chunk'){
        this.chunks.push(new Int16Array(m.buffer));
      }else if(m.type==='end'){
        let n=0; for(const q of this.chunks)n+=q.length;
        this.pcm=new Int16Array(n); let p=0;
        for(const q of this.chunks){this.pcm.set(q,p);p+=q.length;}
        this.chunks=[]; this.totalFrames=Math.floor(this.pcm.length/2); this.ready=true;
      }else if(m.type==='pause'){
        this.paused=true;
      }else if(m.type==='resume'){
        this.paused=false;
      }else if(m.type==='setLoop'){
        this.loopEnabled=!!m.enabled; this.endNotified=false;
      }else if(m.type==='stop'){
        this.chunks=[]; this.pcm=null; this.totalFrames=0; this.ready=false;
        this.sourcePos=0; this.endNotified=false; this.paused=false; this.reportCounter=0;
      }
    };
  }
  frameAt(i){
    if(!this.pcm||i<0||i>=this.totalFrames)return null;
    const p=i*2; return [this.pcm[p],this.pcm[p+1]];
  }
  progressSeconds(){return this.startSeconds+this.sourcePos/this.sourceRate;}
  finish(out,i){
    for(let j=i;j<out[0].length;j++){out[0][j]=0;if(out[1])out[1][j]=0;}
    if(!this.endNotified){this.endNotified=true;this.port.postMessage({type:'ended'});}
  }
  process(_inputs,outputs){
    const out=outputs[0], left=out[0], right=out[1]||out[0];
    if(this.paused||!this.ready){left.fill(0);if(out[1])right.fill(0);return true;}
    const ratio=this.sourceRate/sampleRate;
    const hasValidLoop=this.hasLoop&&this.loopEndFrame!==null&&this.loopEndFrame>0;
    for(let i=0;i<left.length;i++){
      if(hasValidLoop&&this.loopEnabled&&this.sourcePos>=this.loopEndFrame){
        const start=this.loopStartFrame===null?0:this.loopStartFrame;
        const len=this.loopEndFrame-start;
        if(len>0)this.sourcePos=start+((this.sourcePos-this.loopEndFrame)%len);
        this.endNotified=false;
        this.port.postMessage({type:'loop'});
      }
      if(hasValidLoop&&!this.loopEnabled&&this.sourcePos>=this.loopEndFrame){this.finish(out,i);continue;}
      if(!hasValidLoop&&this.sourcePos>=this.totalFrames-1){this.finish(out,i);continue;}
      if(this.sourcePos>=this.totalFrames-1){this.finish(out,i);continue;}

      const base=Math.floor(this.sourcePos), frac=this.sourcePos-base;
      let next=base+1;
      if(hasValidLoop&&this.loopEnabled&&this.loopEndFrame!==null&&next>=this.loopEndFrame){next=this.loopStartFrame===null?0:this.loopStartFrame;}
      const a=this.frameAt(base), b=this.frameAt(next);
      if(!a||!b){this.finish(out,i);continue;}
      left[i]=(a[0]+(b[0]-a[0])*frac)/32768;
      right[i]=(a[1]+(b[1]-a[1])*frac)/32768;
      this.sourcePos+=ratio;
      if(++this.reportCounter>=12){this.reportCounter=0;this.port.postMessage({type:'progress',seconds:this.progressSeconds()});}
    }
    return true;
  }
}
registerProcessor('pcm-player',PcmPlayerProcessor);
