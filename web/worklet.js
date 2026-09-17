class PcmPlayerProcessor extends AudioWorkletProcessor {
  constructor(){
    super();
    this.streams=10;this.chunks=Array.from({length:this.streams},()=>[]);this.totalFrames=0;
    this.ready=false;this.sourcePos=0;this.sourceRate=sampleRate;
    this.ended=false;this.endNotified=false;this.paused=false;this.loopEnabled=false;this.hasLoop=false;
    this.loopStartFrame=0;this.loopEndFrame=0;this.timelineStartSeconds=0;this.reportCounter=0;
    this.muted=Array(this.streams).fill(false);
    this.port.onmessage=e=>{
      const m=e.data||{};
      if(m.type==='init'){
        this.chunks=Array.from({length:this.streams},()=>[]);this.totalFrames=0;this.ready=false;
        this.sourceRate=Number(m.sampleRate)||sampleRate;this.sourcePos=Number(m.initialOffsetFrames)||0;
        this.ended=false;this.endNotified=false;this.paused=false;this.loopEnabled=!!m.loopEnabled;this.hasLoop=!!m.hasLoop;
        this.timelineStartSeconds=Number(m.startSeconds)||0;this.loopStartFrame=Number(m.loopStartFrame)||0;this.loopEndFrame=Number(m.loopEndFrame)||0;this.reportCounter=0;
      }else if(m.type==='chunk'){
        const stream=Number(m.stream);if(stream<0||stream>=this.streams)return;
        const data=new Int16Array(m.buffer);const start=this.chunks[stream].reduce((n,c)=>n+Math.floor(c.data.length/2),0);this.chunks[stream].push({data,startFrame:start});
        if(stream===this.streams-1)this.totalFrames=start+Math.floor(data.length/2);
      }else if(m.type==='end'){this.totalFrames=this.chunks.reduce((max,cs)=>Math.max(max,cs.length?cs[cs.length-1].startFrame+Math.floor(cs[cs.length-1].data.length/2):0),0);if(this.hasLoop){this.loopStartFrame=Math.min(this.loopStartFrame,this.totalFrames);this.loopEndFrame=Math.min(this.loopEndFrame,this.totalFrames);}this.ready=true;this.ended=true;}
      else if(m.type==='pause'){this.paused=true;}
      else if(m.type==='resume'){this.paused=false;}
      else if(m.type==='setLoop'){this.loopEnabled=!!m.enabled;this.endNotified=false;}
      else if(m.type==='mute'){const i=Number(m.stream);if(i>=0&&i<this.streams)this.muted[i]=!!m.muted;}
      else if(m.type==='stop'){this.reset();}
    };
  }
  reset(){this.chunks=Array.from({length:this.streams},()=>[]);this.totalFrames=0;this.ready=false;this.sourcePos=0;this.ended=false;this.endNotified=false;this.paused=false;this.loopEnabled=false;this.hasLoop=false;this.loopStartFrame=0;this.loopEndFrame=0;this.timelineStartSeconds=0;this.reportCounter=0;}
  frameAt(stream,i){
    if(i<0)return null;for(const c of this.chunks[stream]){if(i>=c.startFrame){const p=(i-c.startFrame)*2;if(p+1<c.data.length)return[c.data[p],c.data[p+1]];}else break;}return null;
  }
  finish(left,right,i){left[i]=0;right[i]=0;if(!this.endNotified){this.endNotified=true;this.port.postMessage({type:'ended'});}}
  progressSeconds(){
    const absolute=this.timelineStartSeconds+this.sourcePos/this.sourceRate;
    if(this.hasLoop&&this.loopEnabled&&this.loopEndFrame>this.loopStartFrame){const end=this.timelineStartSeconds+this.loopEndFrame/this.sourceRate;const start=this.timelineStartSeconds+this.loopStartFrame/this.sourceRate;if(absolute>=end){const len=end-start;return start+((absolute-end)%len);}}
    return absolute;
  }
  process(_inputs,outputs){
    const out=outputs[0],left=out[0],right=out[1]||out[0];
    if(this.paused||!this.ready){left.fill(0);if(out[1])right.fill(0);return true;}
    const ratio=this.sourceRate/sampleRate;
    const validLoop=this.hasLoop&&this.loopEndFrame>this.loopStartFrame&&this.loopEndFrame<=this.totalFrames;
    for(let i=0;i<left.length;i++){
      if(validLoop&&this.loopEnabled&&this.sourcePos>=this.loopEndFrame){const len=this.loopEndFrame-this.loopStartFrame;this.sourcePos=this.loopStartFrame+((this.sourcePos-this.loopEndFrame)%len);this.endNotified=false;this.port.postMessage({type:'loop'});}
      if(validLoop&&!this.loopEnabled&&this.sourcePos>=this.loopEndFrame){this.finish(left,right,i);for(let j=i+1;j<left.length;j++){left[j]=0;right[j]=0;}return true;}
      if(!validLoop&&!this.loopEnabled&&this.sourcePos>=this.totalFrames-1){this.finish(left,right,i);for(let j=i+1;j<left.length;j++){left[j]=0;right[j]=0;}return true;}
      if(!validLoop&&this.loopEnabled&&this.sourcePos>=this.totalFrames-1){this.finish(left,right,i);for(let j=i+1;j<left.length;j++){left[j]=0;right[j]=0;}return true;}
      const base=Math.floor(this.sourcePos),frac=this.sourcePos-base;let next=base+1;
      if(validLoop&&this.loopEnabled&&next>=this.loopEndFrame)next=this.loopStartFrame;
      let l=0,r=0;
      for(let s=0;s<this.streams;s++){
        if(this.muted[s])continue;
        const a=this.frameAt(s,base),b=this.frameAt(s,next);if(!a||!b)continue;
        l+=(a[0]+(b[0]-a[0])*frac)/32768;r+=(a[1]+(b[1]-a[1])*frac)/32768;
      }
      left[i]=Math.max(-1,Math.min(1,l));right[i]=Math.max(-1,Math.min(1,r));this.sourcePos+=ratio;
      if(++this.reportCounter>=12){this.reportCounter=0;this.port.postMessage({type:'progress',seconds:this.progressSeconds()});}
    }
    return true;
  }
}
registerProcessor('pcm-player',PcmPlayerProcessor);
