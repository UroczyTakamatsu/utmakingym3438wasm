/*! coi-serviceworker v0.1.7 - Guido Zuidhof and contributors, MIT */
let coepCredentialless = false;
if (typeof window === 'undefined') {
  self.addEventListener('install', () => self.skipWaiting());
  self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
  self.addEventListener('message', ev => {
    if (!ev.data) return;
    if (ev.data.type === 'deregister') {
      self.registration.unregister().then(() => self.clients.matchAll()).then(cs => cs.forEach(c => c.navigate(c.url)));
    } else if (ev.data.type === 'coepCredentialless') {
      coepCredentialless = ev.data.value;
    }
  });
  self.addEventListener('fetch', event => {
    const r = event.request;
    if (r.cache === 'only-if-cached' && r.mode !== 'same-origin') return;
    const request = (coepCredentialless && r.mode === 'no-cors') ? new Request(r, {credentials:'omit'}) : r;
    event.respondWith(fetch(request).then(response => {
      if (response.status === 0) return response;
      const h = new Headers(response.headers);
      h.set('Cross-Origin-Embedder-Policy', coepCredentialless ? 'credentialless' : 'require-corp');
      if (!coepCredentialless) h.set('Cross-Origin-Resource-Policy', 'cross-origin');
      h.set('Cross-Origin-Opener-Policy', 'same-origin');
      return new Response(response.body, {status:response.status,statusText:response.statusText,headers:h});
    }).catch(e => console.error(e)));
  });
} else {
  (() => {
    const reloadedBySelf = window.sessionStorage.getItem('coiReloadedBySelf');
    window.sessionStorage.removeItem('coiReloadedBySelf');
    const coepDegrading = reloadedBySelf === 'coepdegrade';
    const coi = {shouldRegister:()=>!reloadedBySelf,shouldDeregister:()=>false,coepCredentialless:()=>true,coepDegrade:()=>true,doReload:()=>window.location.reload(),quiet:false,...window.coi};
    const n=navigator, controlling=n.serviceWorker&&n.serviceWorker.controller;
    if (controlling && !window.crossOriginIsolated) window.sessionStorage.setItem('coiCoepHasFailed','true');
    const coepHasFailed=window.sessionStorage.getItem('coiCoepHasFailed');
    if (controlling) {
      const reloadToDegrade=coi.coepDegrade() && !(coepDegrading || window.crossOriginIsolated);
      n.serviceWorker.controller.postMessage({type:'coepCredentialless',value:(reloadToDegrade || coepHasFailed&&coi.coepDegrade())?false:coi.coepCredentialless()});
      if(reloadToDegrade){window.sessionStorage.setItem('coiReloadedBySelf','coepdegrade');coi.doReload('coepdegrade');}
      if(coi.shouldDeregister()) n.serviceWorker.controller.postMessage({type:'deregister'});
    }
    if(window.crossOriginIsolated !== false || !coi.shouldRegister()) return;
    if(!window.isSecureContext || !n.serviceWorker) return;
    n.serviceWorker.register(window.document.currentScript.src).then(reg=>{
      reg.addEventListener('updatefound',()=>{window.sessionStorage.setItem('coiReloadedBySelf','updatefound');coi.doReload();});
      if(reg.active&&!n.serviceWorker.controller){window.sessionStorage.setItem('coiReloadedBySelf','notcontrolling');coi.doReload();}
    }).catch(err=>console.error('COOP/COEP Service Worker failed to register:',err));
  })();
}
