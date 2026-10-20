// Synthetic UI only; optional REVIEW_CARD_SOURCE exercises the existing host
// card from a private workspace without copying it into this package.
const {createRequire} = require('node:module');
const fs = require('node:fs');
const assert = require('node:assert/strict');
const req = process.env.PLAYWRIGHT_PACKAGE ? createRequire(process.env.PLAYWRIGHT_PACKAGE) : require;
const {chromium} = req('playwright');
(async () => {
  const browser = await chromium.launch({
    headless:true,
    ...(process.env.PLAYWRIGHT_EXECUTABLE ? {executablePath:process.env.PLAYWRIGHT_EXECUTABLE} : {}),
    args:['--disable-background-networking','--no-first-run']
  });
  try {
    const page = await browser.newPage({viewport:{width:390,height:844}});
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.route('**/*', r => r.request().url() === 'http://fixture.test/'
      ? r.fulfill({contentType:'text/html',body:'<style>body{font-family:system-ui;margin:16px}</style><main></main>'})
      : r.abort());
    await page.goto('http://fixture.test/');
    if (process.env.REVIEW_CARD_SOURCE)
      await page.addScriptTag({type:'module',content:fs.readFileSync(process.env.REVIEW_CARD_SOURCE,'utf8')});
    await page.addScriptTag({type:'module',content:fs.readFileSync('web/review-worker.mjs','utf8')+'\nwindow.attachWorkerReview=attachWorkerReview;'});
    await page.waitForFunction(()=>typeof window.attachWorkerReview==='function');
    await page.evaluate(async real => {
      if (!real) customElements.define('fixture-card',class extends HTMLElement {
        constructor() { super();this.attachShadow({mode:'open'}); }
        async applyRange() {
          this.shadowRoot.innerHTML='<section class="review-editor"><input id="clip-name" value="A moment"><select id="format"><option value="original">Original</option><option value="compatible" selected>Compatible</option></select><button data-action="playable">Legacy prepare</button><button data-action="save">Save</button><p class="playback-help"></p></section>';
        }
      });
      const card=document.createElement(real?'katz-camera-preview':'fixture-card');
      document.querySelector('main').append(card);
      window.card=card;
      if(real) {
        card.setConfig({camera:'fixture',entity:'camera.fixture',title:'Room'});
        card.ready=true;card.base='/synthetic';
        card.api=async(path,body)=>{
          if(body)throw Error('Recorder must not receive an export');
          return path.startsWith('/exports')?[]:[{start_time:0,end_time:Date.now()/1000}];
        };
      } else {card.config={camera:'fixture'};card.range={start:100,end:110};}
      window.jobs=[];window.clips=[];window.posts=[];
      window.transport={
        resolve:async()=> 'synthetic',
        mediaUrl:(id,saved)=>`/synthetic/${saved?'clips':'jobs'}/${id}/media`,
        request:async(path,body)=>{
          if(body)window.posts.push(path);
          if(path==='/jobs/lookup')return structuredClone(window.jobs[0] || null);
          if(path==='/jobs'&&body) {
            const job={id:'a'.repeat(64),request:body,state:'queued'};
            window.jobs=[job];return structuredClone(job);
          }
          if(path==='/jobs')return structuredClone(window.jobs);
          if(path==='/clips')return structuredClone(window.clips);
          if(path.endsWith('/save')) {
            window.clips=[{id:window.jobs[0].id,request:window.jobs[0].request,name:body.name}];
            return structuredClone(window.clips[0]);
          }
          return structuredClone(window.jobs[0]);
        }
      };
      window.attachWorkerReview(card,window.transport);
      if(real) { await card.show('recent');await card.openMoment(Date.now()/1000-120); }
      else await card.applyRange();
    }, Boolean(process.env.REVIEW_CARD_SOURCE));
    const panel=page.locator('katz-worker-review');
    await panel.locator('#prepare').waitFor();
    await panel.locator('#name').fill('The cardboard spaceship');
    await panel.locator('#prepare').click();
    assert.equal(await panel.locator('#save').isDisabled(),true);
    assert.equal(await page.evaluate(()=>window.clips.length),0);
    await page.evaluate(async()=>{
      window.jobs[0].state='ready';
      await window.card.shadowRoot.querySelector('katz-worker-review').session.refresh();
    });
    await panel.locator('#save').click();
    assert.equal(await page.evaluate(()=>window.clips[0].name),'The cardboard spaceship');
    // Remount the same range: restore the job and edited name, never submit work.
    await page.evaluate(()=>window.card.applyRange());
    await page.waitForFunction(()=>window.card.shadowRoot.querySelector('katz-worker-review')?.session.job?.state==='ready');
    assert.equal(await panel.locator('#name').inputValue(),'The cardboard spaceship');
    assert.equal(await page.evaluate(()=>window.posts.filter(p=>p==='/jobs').length),1);
    assert.equal(await page.locator('#format').inputValue(),'original');
    assert.equal(await page.locator('[data-action="playable"]').isVisible(),false);
    await panel.locator('#load').click();
    await panel.locator('#library button').click();
    assert.match(await panel.locator('video').getAttribute('src'),/\/clips\//);
    // Detach during a pending operation: reconnect restores rather than staying busy.
    await page.evaluate(async()=>{
      const p=window.card.shadowRoot.querySelector('katz-worker-review'), parent=p.parentNode;
      const original=window.transport.request;
      let finish;
      window.transport.request=()=>new Promise(r=>finish=r);
      const saving=p.session.save('Pending');
      p.remove();window.transport.request=original;parent.append(p);
      finish({id:'a'.repeat(64),name:'Pending'});await saving;
    });
    await page.waitForFunction(()=>{
      const s=window.card.shadowRoot.querySelector('katz-worker-review').session;
      return !s.busy && s.job?.state==='ready';
    });
    // A distinct selection gets its own draft, never the previous moment's name.
    await page.evaluate(async()=>{
      const input=window.card.shadowRoot.querySelector('#clip-start');
      if(input) input.stepUp(60);
      else window.card.range={...window.card.range,start:window.card.range.start+1};
      await window.card.applyRange();
    });
    assert.equal(await panel.locator('#name').inputValue(),'A moment');
    await page.evaluate(async()=>{
      const p=window.card.shadowRoot.querySelector('katz-worker-review');
      const original=window.transport.mediaUrl;
      let finish;
      window.transport.mediaUrl=()=>new Promise(r=>finish=r);
      const old=p.play('b'.repeat(64),false);
      window.transport.mediaUrl=original;
      await p.play('c'.repeat(64),false);
      finish('/synthetic/old-signed-url');await old;
    });
    assert.match(await panel.locator('video').getAttribute('src'),/c{64}/);
    for(const width of [390,768,1440]) {
      await page.setViewportSize({width,height:900});
      assert.equal(await panel.evaluate(e=>e.scrollWidth<=e.clientWidth+1),true,`overflow at ${width}`);
    }
    assert.deepEqual(errors,[]);
    console.log(`Browser review/save, navigation recovery, name retention, original export seam, library reopen and responsive widths passed (${process.env.REVIEW_CARD_SOURCE?'actual host card':'fixture host'}; mocked media/API)`);
  } finally { await browser.close(); }
})().catch(e=>{console.error(e);process.exitCode=1;});
