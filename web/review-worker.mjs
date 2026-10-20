// Transport is supplied by the authenticated host. No worker/NAS credential or
// arbitrary source path belongs in this component.
export class ReviewSession {
  constructor(transport, onChange = () => {}) {
    this.transport = transport; this.onChange = onChange;
    this.generation = 0; this.job = null; this.busy = false;
  }
  select(selection) {
    this.generation++; this.selection = {...selection}; this.job = null;
    this.busy = false; this.message = ''; this.saved = null; this.onChange(this);
  }
  async restore() {
    const generation = this.generation, selection = {...this.selection};
    this.busy = true; this.message = 'Checking for an existing preview…'; this.onChange(this);
    try {
      const source = await this.transport.resolve(selection);
      if (generation !== this.generation) return;
      const job = await this.transport.request('/jobs/lookup', {
        source, ...selection, profile:'browser-1080p-v1'
      });
      if (generation !== this.generation) return;
      this.job = job;
      this.message = '';
    } catch {
      if (generation === this.generation) this.message = 'Could not check for an existing preview. Prepare preview will safely reuse any matching job.';
    } finally {
      if (generation === this.generation) { this.busy = false; this.onChange(this); }
    }
  }
  async prepare() {
    if (this.busy) return;
    const generation = this.generation, selection = {...this.selection};
    this.busy = true; this.message = 'Finding this recording…'; this.onChange(this);
    try {
      if (['failed','cancelled'].includes(this.job?.state)) {
        const job = await this.transport.request(`/jobs/${this.job.id}/retry`, {});
        if (generation === this.generation) { this.job = job; this.message = ''; }
        return;
      }
      const source = await this.transport.resolve(selection);
      if (generation !== this.generation) return;
      const job = await this.transport.request('/jobs', {
        source, ...selection, profile: 'browser-1080p-v1'
      });
      if (generation !== this.generation) return;
      this.job = job; this.message = '';
    } catch {
      if (generation === this.generation)
        this.message = 'Could not prepare this moment. Your selection is still here; you can retry.';
    } finally {
      if (generation === this.generation) { this.busy = false; this.onChange(this); }
    }
  }
  async refresh() {
    if (!this.job || this.busy) return;
    const generation = this.generation, id = this.job.id;
    try {
      const job = await this.transport.request(`/jobs/${id}`);
      if (generation !== this.generation || id !== this.job?.id) return;
      this.job = job; this.message = '';
    } catch {
      if (generation === this.generation) this.message = 'Connection interrupted. Your preparation may still be running; checking again shortly.';
    }
    if (generation === this.generation) this.onChange(this);
  }
  async cancel() {
    if (!this.job || this.busy || !['queued','running'].includes(this.job.state)) return;
    const generation = this.generation, id = this.job.id;
    try {
      await this.transport.request(`/jobs/${id}/cancel`, {});
      if (generation === this.generation) this.message = 'Stopping preparation…';
    } catch {
      if (generation === this.generation) this.message = 'Stop could not be confirmed. Checking preparation status.';
    }
    if (generation === this.generation) this.onChange(this);
  }
  async save(name) {
    if (this.busy || this.job?.state !== 'ready') return;
    const generation = this.generation, id = this.job.id;
    this.busy = true; this.message = 'Saving your clip…'; this.onChange(this);
    try {
      const clip = await this.transport.request(`/jobs/${id}/save`, {name});
      if (generation !== this.generation) return;
      this.saved = clip; this.message = 'Saved. Your named clip is ready to reopen.';
    } catch {
      if (generation === this.generation) this.message = 'Save could not be confirmed. Retry to check and reuse the same clip.';
    } finally {
      if (generation === this.generation) { this.busy = false; this.onChange(this); }
    }
  }
}

export function sameOriginTransport(base, resolve) {
  if (!base.startsWith('/') || base.startsWith('//') || /[?#\\]/.test(base))
    throw Error('Authenticated same-origin worker route required');
  return {
    resolve,
    async request(path, body) {
      const response = await fetch(base + path, {
        method: body === undefined ? 'GET' : 'POST',
        credentials: 'same-origin', redirect: 'error', cache: 'no-store',
        headers: {'Content-Type':'application/json', 'X-CSRF-Token':'1'},
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(15000)
      });
      if (!response.ok) throw Error('Worker request failed');
      return response.json();
    },
    mediaUrl(id, saved = false) {
      if (!/^[a-f0-9]{64}$/.test(id)) throw Error('Invalid clip identity');
      return `${base}/${saved ? 'clips' : 'jobs'}/${id}/media`;
    }
  };
}

if (typeof HTMLElement !== 'undefined') {
  class WorkerReview extends HTMLElement {
    constructor() {
      super(); this.attachShadow({mode:'open'});
      this.shadowRoot.innerHTML = `<style>
        :host{display:block;color:inherit;font:inherit}button,input{font:inherit}
        .row{display:flex;flex-wrap:wrap;gap:.75rem;align-items:end}
        button,input{padding:.75rem;border-radius:.6rem;border:1px solid #687480}
        button{cursor:pointer;background:#213447;color:#fff}button:disabled{opacity:.5;cursor:default}
        label{display:grid;gap:.4rem;flex:1;min-width:10rem}input{background:transparent;color:inherit;min-width:0}
        p{line-height:1.5}video{width:100%;max-height:65vh;background:#000;border-radius:.6rem}
        #status{min-height:1.5em}#library button{margin:.25rem}small{display:block;margin:.6rem 0;opacity:.8}
      </style><section aria-label="Playable moment">
        <div class="row"><button id="prepare">Prepare preview</button><button id="cancel" hidden>Stop preparation</button></div>
        <p id="status" role="status" aria-live="polite"></p><div id="player"></div>
        <small id="detail">Preparing a preview does not save a permanent clip.</small>
        <div class="row"><label>Clip name<input id="name" maxlength="120"></label><button id="save" disabled>Save clip</button></div>
        <p><button id="load">Open saved clips</button></p><div id="library"></div>
      </section>`;
      this.$('#prepare').onclick = () => this.session.prepare();
      this.$('#cancel').onclick = () => this.session.cancel();
      this.$('#save').onclick = () => this.session.save(this.$('#name').value);
      this.$('#load').onclick = () => this.library();
      this.$('#name').oninput = () => {
        try { sessionStorage.setItem(this.nameKey, this.$('#name').value); } catch { /* optional */ }
      };
    }
    $(selector) { return this.shadowRoot.querySelector(selector); }
    configure(transport, selection, name) {
      this.transport = transport;
      this.session = new ReviewSession(transport, () => this.render());
      this.session.select(selection);
      this.nameKey = `video-clip-name:${JSON.stringify(selection)}`;
      let remembered;
      try { remembered = sessionStorage.getItem(this.nameKey); } catch { /* optional */ }
      this.$('#name').value = remembered ?? name;
      this.connect();
      this.session.restore();
    }
    connectedCallback() { this.connect(); }
    disconnectedCallback() { clearInterval(this.timer); this.session && this.session.generation++; }
    connect() {
      clearInterval(this.timer);
      if (this.isConnected && this.session) this.timer = setInterval(() => {
        if (['queued','running'].includes(this.session.job?.state)) this.session.refresh();
      }, 1000);
    }
    render() {
      const s = this.session, state = s.job?.state;
      this.$('#status').textContent = s.message || ({
        queued:'Your preview is queued.', running:'Preparing your preview… You can keep editing the name.',
        ready:'Preview ready. Review it, then save it if you want to keep it.',
        failed:'Preparation failed. No clip was saved.',
        cancelled:'Preparation stopped. No clip was saved.'
      }[state] || 'Choose Prepare preview to review this selection.');
      this.$('#prepare').disabled = s.busy || ['queued','running','ready'].includes(state);
      this.$('#prepare').textContent = ['failed','cancelled'].includes(state) ? 'Retry preparation' : 'Prepare preview';
      this.$('#cancel').hidden = !['queued','running'].includes(state);
      this.$('#save').disabled = s.busy || state !== 'ready';
      if (state === 'ready') this.play(s.job.id, false);
    }
    play(id, saved) {
      const url = this.transport.mediaUrl(id, saved);
      if (this.$('video')?.getAttribute('src') === url) return;
      this.$('video')?.pause();
      const video = document.createElement('video');
      video.controls = true; video.playsInline = true; video.preload = 'metadata'; video.src = url;
      video.onerror = () => { this.$('#status').textContent = 'This copy could not play. Your saved clip, if any, is still available.'; };
      this.$('#player').replaceChildren(video);
    }
    async library() {
      const session = this.session, generation = session.generation;
      try {
        const clips = await this.transport.request('/clips');
        if (!this.isConnected || session !== this.session || generation !== session.generation) return;
        this.$('#library').replaceChildren();
        for (const clip of clips.filter(c => c.request.camera === session.selection.camera)) {
          const button = document.createElement('button'); button.textContent = clip.name;
          button.onclick = () => {
            this.play(clip.id, true); this.$('#status').textContent = clip.name;
          };
          this.$('#library').append(button);
        }
        if (!this.$('#library').childElementCount) this.$('#library').textContent = 'No saved clips yet.';
      } catch { this.$('#status').textContent = 'Saved clips could not be loaded. Try again.'; }
    }
  }
  customElements.define('katz-worker-review', WorkerReview);
}

// Explicit opt-in seam for the current review card. Recorder functionality is
// unchanged when no authenticated worker transport has been attached.
export function attachWorkerReview(card, transport) {
  if (card.workerReviewAttached) throw Error('Worker review is already attached');
  const original = card.applyRange;
  if (typeof original !== 'function') throw Error('Review card does not expose applyRange');
  card.workerReviewAttached = true;
  let selectionVersion = 0;
  card.applyRange = async function(...args) {
    const version = ++selectionVersion;
    const priorName = this.shadowRoot.querySelector('katz-worker-review')?.shadowRoot.querySelector('#name').value;
    this.shadowRoot.querySelector('katz-worker-review')?.remove();
    await original.apply(this,args);
    if (version !== selectionVersion) return;
    if (!this.range || !this.isConnected) {
      this.shadowRoot.querySelector('katz-worker-review')?.remove();
      return;
    }
    const stage = this.shadowRoot.querySelector('.review-editor');
    if (!stage) return;
    stage.querySelector('katz-worker-review')?.remove();
    const panel = document.createElement('katz-worker-review');
    stage.append(panel);
    panel.configure(transport, {
      camera:this.config.camera, start_ms:Math.round(this.range.start*1000),
      end_ms:Math.round(this.range.end*1000)
    }, priorName || this.shadowRoot.querySelector('#clip-name')?.value || 'A moment');
    // Existing original-quality save remains available; prevent duplicate
    // legacy software conversion once this panel supplies compatible playback.
    const legacy = this.shadowRoot.querySelector('[data-action="playable"]');
    if (legacy) legacy.hidden = true;
    const option = this.shadowRoot.querySelector('#format option[value="compatible"]');
    if (option) option.disabled = true;
    const format = this.shadowRoot.querySelector('#format');
    if (format) format.value = 'original';
    const save = this.shadowRoot.querySelector('[data-action="save"]');
    if (save) save.textContent = 'Save original';
    const help = this.shadowRoot.querySelector('.playback-help');
    if (help) help.textContent = 'Prepare a browser preview below, or save the original-quality recording.';
    panel.shadowRoot.addEventListener('play', () => {
      for (const video of this.shadowRoot.querySelectorAll('video')) video.pause();
    }, true);
  };
}
