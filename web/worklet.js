class PcmPlayerProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks=[];
    this.pcm=null;
    this.totalFrames=0;
    this.sourceRate=sampleRate;
    this.sourcePos=0;
    this.startSeconds=0;
    this.loopEnabled=false;
    this.hasLoop=false;
    this.loopStartFrame=null;
    this.loopEndFrame=null;
    this.started=false;
    this.paused=false;
    this.endNotified=false;
    this.reportCounter=0;

    this.port.onmessage=e=>{
      const m=e.data||{};
      if(m.type==='init'){
        this.chunks=[];
        this.pcm=null;
        this.totalFrames=Number(m.totalFrames)||0;
        this.sourceRate=Number(m.sampleRate)||sampleRate;
        this.sourcePos=0;
        this.startSeconds=Number(m.startSeconds)||0;
        this.loopEnabled=!!m.loopEnabled;
        this.hasLoop=!!m.hasLoop;
        this.loopStartFrame=Number.isFinite(Number(m.loopStartFrame))?Number(m.loopStartFrame):null;
        this.loopEndFrame=Number.isFinite(Number(m.loopEndFrame))?Number(m.loopEndFrame):null;
        this.started=false;
        this.paused=false;
        this.endNotified=false;
        this.reportCounter=0;
      }else if(m.type==='chunk'){
        this.chunks.push(new Int16Array(m.buffer));
      }else if(m.type==='end'){
        let length=0;
        for(const q of this.chunks) length+=q.length;
        this.pcm=new Int16Array(length);
        let p=0;
        for(const q of this.chunks){this.pcm.set(q,p);p+=q.length;}
        this.chunks=[];
        this.totalFrames=Math.floor(this.pcm.length/2);
        this.started=true;
      }else if(m.type==='pause'){
        this.paused=true;
      }else if(m.type==='resume'){
        this.paused=false;
      }else if(m.type==='setLoop'){
        this.loopEnabled=!!m.enabled;
      }else if(m.type==='stop'){
        this.chunks=[];
        this.pcm=null;
        this.sourcePos=0;
        this.started=false;
        this.paused=false;
        this.endNotified=false;
        this.reportCounter=0;
      }
    };
  }

  frameAt(frame){
    if(!this.pcm || frame<0 || frame>=this.totalFrames) return null;
    const p=frame*2;
    return [this.pcm[p],this.pcm[p+1]];
  }

  songSeconds(){
    return this.startSeconds + this.sourcePos/this.sourceRate;
  }

  finish(left,right,i){
    for(let j=i;j<left.length;j++){left[j]=0;right[j]=0;}
    if(!this.endNotified){
      this.endNotified=true;
      this.port.postMessage({type:'ended'});
    }
  }

  process(_inputs,outputs){
    const output=outputs[0];
    const left=output[0];
    const right=output[1]||output[0];

    if(this.paused || !this.started || !this.pcm){
      for(const ch of output) ch.fill(0);
      return true;
    }

    const ratio=this.sourceRate/sampleRate;

    for(let i=0;i<left.length;i++){
      if(this.hasLoop && this.loopEnabled && this.loopEndFrame!==null && this.sourcePos>=this.loopEndFrame){
        this.sourcePos=this.loopStartFrame!==null ? this.loopStartFrame : 0;
      }

      if(this.hasLoop && !this.loopEnabled && this.loopEndFrame!==null && this.sourcePos>=this.loopEndFrame){
        this.finish(left,right,i); continue;
      }
      if(!this.hasLoop && this.sourcePos>=this.totalFrames){
        this.finish(left,right,i); continue;
      }

      const base=Math.floor(this.sourcePos);
      const frac=this.sourcePos-base;
      let next=base+1;
      if(this.hasLoop && this.loopEnabled && this.loopEndFrame!==null && next>=this.loopEndFrame){
        next=this.loopStartFrame!==null ? this.loopStartFrame : 0;
      }
      const a=this.frameAt(base);
      const b=this.frameAt(next);
      if(!a || !b){this.finish(left,right,i);continue;}

      left[i]=(a[0]+(b[0]-a[0])*frac)/32768;
      right[i]=(a[1]+(b[1]-a[1])*frac)/32768;
      this.sourcePos+=ratio;
      if(++this.reportCounter>=12){
        this.reportCounter=0;
        this.port.postMessage({type:'progress',seconds:this.songSeconds()});
      }
    }
    return true;
  }
}
registerProcessor('pcm-player',PcmPlayerProcessor);
