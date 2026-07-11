importScripts("config.js", "repo-identity.js");
(() => {
  "use strict";
  const C=globalThis.SEVAL_CONFIG, TERMINAL=new Set(["completed","completed_partial","failed"]), STATES=new Set(["queued","resolving","downloading","extracting","analyzing",...TERMINAL]);
  const FORBIDDEN=new Set(["score","quality_score","verdict","grade","winner"]);
  function validSender(s){if(!s||s.id!==chrome.runtime.id||typeof s.url!=="string")return false;try{const u=new URL(s.url);return u.protocol==="https:"&&u.hostname==="github.com";}catch(_){return false;}}
  function buildAnalysisUrl(id){const path=id===undefined?"/v1/analyses":`/v1/analyses/${encodeURIComponent(id)}`;const u=new URL(path,`${C.apiOrigin}/`);if(u.origin!==C.apiOrigin)throw new Error("Invalid API destination");return u.href;}
  function clampPollDelay(ms){const n=Number(ms);return Number.isFinite(n)?Math.max(C.pollMinDelayMs,Math.min(C.pollMaxDelayMs,n)):C.pollMinDelayMs;}
  function containsForbiddenJudgmentKeys(value){if(!value||typeof value!=="object")return false;if(Array.isArray(value))return value.some(containsForbiddenJudgmentKeys);return Object.entries(value).some(([k,v])=>FORBIDDEN.has(k.toLowerCase())||containsForbiddenJudgmentKeys(v));}
  function adaptServiceResponse(v){
    if(!v||typeof v!=="object"||Array.isArray(v)||typeof v.state!=="string"||!STATES.has(v.state))throw new Error("Invalid service response");
    const adapted={analysis_id:v.analysis_id,status:v.state};
    if(!v.result)return adapted;
    const compact=v.result;
    if(typeof compact!=="object"||Array.isArray(compact))throw new Error("Invalid service response");
    const repository=compact.repository&&typeof compact.repository==="object"&&!Array.isArray(compact.repository)?compact.repository:{};
    const keyed=compact.instruments&&typeof compact.instruments==="object"&&!Array.isArray(compact.instruments)?compact.instruments:{};
    const metrics=Object.keys(keyed).sort().slice(0,6).map(id=>{
      const item=keyed[id]&&typeof keyed[id]==="object"&&!Array.isArray(keyed[id])?keyed[id]:{};
      const metric={id,analyzer:item.analyzer,status:item.state,coverage:item.coverage,observations:item.observations,limitations:Array.isArray(item.limitations)?item.limitations:[]};
      if(item.error!==undefined)metric.error=item.error;
      return metric;
    });
    const snapshot={full_name:repository.full_name,repository_id:repository.repository_id,commit:repository.commit,cache:{hit:Boolean(repository.cached)}};
    adapted.result={snapshot,metrics};
    return adapted;
  }
  function normalizeResponse(v){
    if(!v||typeof v!=="object"||Array.isArray(v)||containsForbiddenJudgmentKeys(v))throw new Error("Invalid service response");
    return typeof v.state==="string"?adaptServiceResponse(v):v;
  }
  async function boundedJson(id,options,timeout){const ctl=new AbortController(),timer=setTimeout(()=>ctl.abort(),timeout);try{const r=await fetch(buildAnalysisUrl(id),{...options,redirect:"error",credentials:"omit",cache:"no-store",referrerPolicy:"no-referrer",signal:ctl.signal});const len=Number(r.headers.get("content-length"));if(Number.isFinite(len)&&len>C.maxResponseBytes)throw new Error("Response too large");const text=await r.text();if(new TextEncoder().encode(text).byteLength>C.maxResponseBytes)throw new Error("Response too large");let body;try{body=normalizeResponse(JSON.parse(text));}catch(_){throw new Error("Invalid service response");}if(!r.ok){const e=new Error("Analysis request failed.");e.retryable=Boolean(body.error&&body.error.retryable);throw e;}return body;}finally{clearTimeout(timer);}}
  const delay=(ms)=>new Promise(r=>setTimeout(r,ms));
  async function analyze(identity,emit){let cur=await boundedJson(undefined,{method:"POST",headers:{"Content-Type":"application/json",Accept:"application/json"},body:JSON.stringify(identity)},C.requestTimeoutMs);if(TERMINAL.has(cur.status))return cur;if(typeof cur.analysis_id!=="string"||!/^[A-Za-z0-9_-]{1,128}$/.test(cur.analysis_id))throw new Error("Invalid analysis identifier");const id=cur.analysis_id,start=Date.now();let wait=C.pollMinDelayMs;for(let i=0;i<C.maxPollAttempts&&Date.now()-start<C.pollTimeoutMs;i+=1){emit({type:"progress",result:cur});await delay(clampPollDelay(cur.retry_after_ms??wait));cur=await boundedJson(id,{method:"GET",headers:{Accept:"application/json"}},C.requestTimeoutMs);if(TERMINAL.has(cur.status))return cur;wait=clampPollDelay(wait*1.5);}throw new Error("Analysis polling limit reached");}
  if(globalThis.__SEVAL_EXTENSION_TEST__)globalThis.SEVAL_BACKGROUND_TEST=Object.freeze({validSender,buildAnalysisUrl,clampPollDelay,adaptServiceResponse,normalizeResponse,containsForbiddenJudgmentKeys,analyze});
  chrome.runtime.onConnect.addListener(port=>{if(port.name!=="seval-analysis"||!validSender(port.sender)){port.disconnect();return;}let closed=false,started=false;port.onDisconnect.addListener(()=>{closed=true;});port.onMessage.addListener(m=>{if(closed||started||!m||m.type!=="analyze"||m.requestedIdentity!==true)return;started=true;const id=globalThis.SEVAL_REPO_IDENTITY.parseNwo(`${m.owner||""}/${m.repo||""}`);if(!id){port.postMessage({type:"error",message:"This repository identity is not valid."});return;}analyze({owner:id.owner.toLowerCase(),repo:id.repo.toLowerCase()},x=>{if(!closed)port.postMessage(x);}).then(result=>{if(!closed)port.postMessage({type:"result",result});}).catch(e=>{if(!closed)port.postMessage({type:"error",message:e&&e.name==="AbortError"?"The analysis service took too long to respond.":"The analysis service could not complete this request.",retryable:Boolean(e&&e.retryable)});});});});
})();
