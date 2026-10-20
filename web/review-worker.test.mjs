import assert from 'node:assert/strict';
import {ReviewSession,sameOriginTransport} from './review-worker.mjs';
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
console.log('Review preparation/save separation, stale navigation, cancellation and transport tests passed');
