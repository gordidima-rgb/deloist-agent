// Отмена и повтор правок: снимки графа строкой, не больше `limit` шагов назад.
export class History {
  constructor(limit = 100) {
    this.limit = limit;
    this.past = [];
    this.future = [];
    this.head = null;
  }

  // Новое начальное состояние (загрузили процесс): история чистая.
  reset(snapshot) {
    this.past = [];
    this.future = [];
    this.head = snapshot;
  }

  // Запомнить состояние после правки. Одинаковое с последним — не шаг.
  push(snapshot) {
    if (snapshot === this.head) return false;
    if (this.head !== null) this.past.push(this.head);
    if (this.past.length > this.limit) this.past.shift();
    this.head = snapshot;
    this.future = [];
    return true;
  }

  get canUndo() {
    return this.past.length > 0;
  }

  get canRedo() {
    return this.future.length > 0;
  }

  undo() {
    if (!this.past.length) return null;
    this.future.push(this.head);
    this.head = this.past.pop();
    return this.head;
  }

  redo() {
    if (!this.future.length) return null;
    this.past.push(this.head);
    this.head = this.future.pop();
    return this.head;
  }
}
