// Сохранение графа: одна отложенная запись, не больше одной в полёте, ревизия из ответа.
// Ответ 409 — кто-то сохранил раньше: отдаём свежий процесс наружу и больше не пишем эту версию.
export class Saver {
  /**
   * api(path, {method, body}); id; revision — текущая; getGraph() — что писать;
   * on: { saved(revision), conflict(workflow), error(err), state(name) } — name: idle|dirty|saving|error
   */
  constructor({ api, id, revision, getGraph, on = {}, delay = 800 }) {
    this.api = api;
    this.id = id;
    this.revision = revision;
    this.getGraph = getGraph;
    this.on = on;
    this.delay = delay;
    this.timer = 0;
    this.inFlight = null;
    this.again = false;
    this.dead = false;
  }

  schedule() {
    if (this.dead) return;
    this.on.state?.('dirty');
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = 0;
      this.flush();
    }, this.delay);
  }

  get pending() {
    return Boolean(this.timer) || Boolean(this.inFlight) || this.again;
  }

  async flush() {
    if (this.dead) return;
    clearTimeout(this.timer);
    this.timer = 0;
    if (this.inFlight) {
      this.again = true;
      return this.inFlight;
    }
    this.inFlight = this.#write();
    try {
      await this.inFlight;
    } finally {
      this.inFlight = null;
    }
    if (this.again && !this.dead) {
      this.again = false;
      await this.flush();
    }
  }

  async #write() {
    this.on.state?.('saving');
    try {
      const res = await this.api(`/api/workflows/${encodeURIComponent(this.id)}/graph`, {
        method: 'PUT',
        body: { base_revision: this.revision, graph: this.getGraph() },
      });
      if (typeof res?.revision === 'number') this.revision = res.revision;
      this.on.saved?.(this.revision);
      this.on.state?.(this.timer || this.again ? 'dirty' : 'idle');
    } catch (err) {
      if (err?.status === 409) {
        this.dead = true;
        this.on.conflict?.(err.data?.workflow ?? null);
        this.on.state?.('idle');
      } else {
        this.on.error?.(err);
        this.on.state?.('error');
      }
    }
  }

  // Процесс закрывают: дописать то, что не записано, и выйти.
  async close() {
    if (this.timer || this.again) await this.flush();
    else if (this.inFlight) await this.inFlight;
    this.dead = true;
  }

  destroy() {
    clearTimeout(this.timer);
    this.timer = 0;
    this.dead = true;
  }
}
