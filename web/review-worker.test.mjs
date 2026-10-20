import assert from 'node:assert/strict';
import {ReviewSession,sameOriginTransport,homeAssistantTransport} from './review-worker.mjs';
const selection={camera:'room',start_ms:11000,end_ms:13000};
const id='a'.repeat(64);
let state='queued', saved=0, calls=[];
const transport={
  resolve:async()=> 'fixture',
  request:async(path,body)=>{
    calls.push({path,body});
    if(path.endsWith('/save')) { saved++;return {id,name:body.name}; }
    if(path.endsWith('/cancel')) return {cancel_requested:true};
    return {id,state};
  }
};
const session=new ReviewSession(transport);
session.select(selection);await session.prepare();
assert.equal(session.job.state,'queued');assert.equal(saved,0);
await session.save('Too early');assert.equal(saved,0);
state='ready';await session.refresh();await session.save('A small discovery');
assert.equal(saved,1);assert.equal(session.saved.name,'A small discovery');
assert.equal(calls[0].body.profile,'browser-1080p-v1');
// Navigation during source resolution must not start an abandoned selection.
let resolve;const delayed={...transport,resolve:()=>new Promise(r=>resolve=r)};
const stale=new ReviewSession(delayed);stale.select(selection);const preparing=stale.prepare();
const count=calls.length;stale.select({...selection,start_ms:12000});resolve('fixture');await preparing;
assert.equal(calls.length,count);assert.equal(stale.job,null);assert.equal(stale.busy,false);
// Save acknowledgment for the old selection cannot label the new selection saved.
let complete;const saving=new ReviewSession({...transport,request:()=>new Promise(r=>complete=r)});
saving.select(selection);saving.job={id,state:'ready'};const pending=saving.save('Name');
saving.select({...selection,end_ms:14000});complete({id,name:'Name'});await pending;
assert.equal(saving.saved,null);
state='running';session.select(selection);await session.prepare();await session.cancel();
assert.equal(session.job.state,'running');assert.match(session.message,/Stopping/);
state='cancelled';await session.refresh();assert.equal(session.job.state,'cancelled');
assert.throws(()=>sameOriginTransport('http://worker:8787',()=>{}));
assert.throws(()=>sameOriginTransport('//evil',()=>{}));
assert.throws(()=>sameOriginTransport('/worker',()=>{}).mediaUrl('../x'));
const restores=[];
const restored=new ReviewSession({...transport,request:async(path,body)=>{
  restores.push({path,body});return {id,state:'ready'};
}});
restored.select(selection);await restored.restore();
assert.equal(restored.job.id,id);
assert.deepEqual(restores.map(c=>c.path),['/jobs/lookup']);
assert.equal(restores[0].body.source,'fixture');
await restored.save('   ');
assert.equal(restores.length,1);
assert.match(restored.message,/Give this clip a name/);
// An old lookup must not restore footage into the newly selected moment.
let finishLookup;
const moving=new ReviewSession({...transport,request:()=>new Promise(r=>finishLookup=r)});
moving.select(selection);const looking=moving.restore();await Promise.resolve();
moving.select({...selection,end_ms:14000});finishLookup({id,state:'ready'});await looking;
assert.equal(moving.job,null);
// Slow polls cannot overlap and overwrite a later terminal result.
let finishRefresh, refreshCalls=0;
const polling=new ReviewSession({...transport,request:()=>{
  refreshCalls++; return new Promise(r=>finishRefresh=r);
}});
polling.select(selection);polling.job={id,state:'running'};
const firstPoll=polling.refresh();await polling.refresh();
assert.equal(refreshCalls,1);
finishRefresh({id,state:'ready'});await firstPoll;
assert.equal(polling.job.state,'ready');
assert.equal(polling.refreshing,null);
const haCalls=[];
const ha=homeAssistantTransport({callApi:async(...args)=>{
  haCalls.push(args);return {url:`/api/ha_video_worker/media/jobs/${id}/room/user?authSig=synthetic`};
}});
await ha.request('/jobs',{...selection,source:'untrusted',profile:'arbitrary'});
assert.deepEqual(haCalls[0],['POST','ha_video_worker/control/jobs',selection]);
assert.match(await ha.mediaUrl(id),/^\/api\/ha_video_worker\/media\//);
assert.deepEqual(haCalls[1],['POST','ha_video_worker/media-url',{id,kind:'jobs'}]);
assert.throws(()=>ha.request('/jobs/../secret'));
await assert.rejects(homeAssistantTransport({callApi:async()=>({url:'https://elsewhere/secret'})}).mediaUrl(id));
console.log('Review preparation/save separation, stale navigation, cancellation and transport tests passed');
