// CI-only synthetic bridge. This deliberately is NOT a production auth proxy.
const http = require('node:http');
const fs = require('node:fs');
const assert = require('node:assert/strict');
const {createRequire} = require('node:module');
const req = process.env.PLAYWRIGHT_PACKAGE ? createRequire(process.env.PLAYWRIGHT_PACKAGE) : require;
const {chromium} = req('playwright');
(async()=>{
  const base=process.env.VIDEO_TEST_BASE, token=process.env.VIDEO_TEST_TOKEN;
  assert.match(base,/^http:\/\/127\.0\.0\.1:\d+$/);
  const server=http.createServer(async(request,response)=>{
    try {
      if(request.url==='/') {
        response.setHeader('Content-Type','text/html');
        response.end('<style>body{font:18px system-ui;margin:24px;background:#f4f3ef;color:#213447;max-width:900px}</style><h1>Review a moment</h1><katz-worker-review></katz-worker-review><script type="module">import {sameOriginTransport} from "/review.mjs";document.querySelector("katz-worker-review").configure(sameOriginTransport("/worker"),{camera:"fixture",start_ms:12000,end_ms:14000},"A synthetic moment");</script>');
      } else if(request.url==='/review.mjs') {
        response.setHeader('Content-Type','text/javascript');
        response.end(fs.readFileSync('web/review-worker.mjs'));
      } else if(request.url.startsWith('/worker/')) {
        const chunks=[];for await(const chunk of request)chunks.push(chunk);
        const body=Buffer.concat(chunks);
        const upstream=await fetch(base+request.url.slice('/worker'.length),{
          method:request.method,
          headers:{Authorization:`Bearer ${token}`, ...(request.headers.range?{Range:request.headers.range}:{})},
          body:body.length?body:undefined,signal:AbortSignal.timeout(10000),redirect:'error'
        });
        response.statusCode=upstream.status;
        for(const key of ['content-type','content-range','accept-ranges','cache-control'])
          if(upstream.headers.has(key))response.setHeader(key,upstream.headers.get(key));
        response.end(Buffer.from(await upstream.arrayBuffer()));
      } else {response.statusCode=404;response.end();}
    } catch {response.statusCode=502;response.end();}
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=await chromium.launch({headless:true,args:['--autoplay-policy=no-user-gesture-required']});
  try {
    const page=await browser.newPage({viewport:{width:390,height:844}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    const panel=page.locator('katz-worker-review');
    await panel.locator('#prepare').click();
    await page.waitForFunction(()=>document.querySelector('katz-worker-review')?.session.job?.state==='ready',{},{timeout:20000});
    const video=panel.locator('video');
    await video.evaluate(v=>v.play());
    await page.waitForFunction(()=>{
      const v=document.querySelector('katz-worker-review').shadowRoot.querySelector('video');
      return v.videoWidth===320&&v.videoHeight===180&&v.currentTime>.25;
    });
    await video.evaluate(v=>{v.currentTime=1;});
    await page.waitForFunction(()=>document.querySelector('katz-worker-review').shadowRoot.querySelector('video').currentTime>1.1);
    await panel.locator('#name').fill('A browser-reviewed moment');
    await panel.locator('#save').click();
    await page.waitForFunction(()=>document.querySelector('katz-worker-review').session.saved?.name==='A browser-reviewed moment');
    await panel.locator('#load').click();
    await panel.getByRole('button',{name:'A browser-reviewed moment',exact:true}).click();
    assert.match(await video.getAttribute('src'),/\/clips\//);
    await video.evaluate(v=>v.play());
    await page.waitForFunction(()=>document.querySelector('katz-worker-review').shadowRoot.querySelector('video').currentTime>.25);
    // Reload uses the current source identity to restore, not a fresh conversion.
    const id=await panel.evaluate(e=>e.session.job.id);
    await page.reload();
    await page.waitForFunction(()=>document.querySelector('katz-worker-review')?.session.job?.state==='ready');
    assert.equal(await panel.evaluate(e=>e.session.job.id),id);
    assert.equal(await panel.locator('#name').inputValue(),'A browser-reviewed moment');
    assert.deepEqual(errors,[]);
    console.log('Real browser: prepare, decoded video dimensions, playback advancement, seek, save, saved playback and reload reuse passed');
  } finally {
    await browser.close();
    await new Promise(resolve=>server.close(resolve));
  }
})().catch(e=>{console.error(e);process.exitCode=1;});
