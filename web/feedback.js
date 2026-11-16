// Окно «Ошибка или пожелание»: человек пишет прямо в программе, без аккаунта на GitHub.
window.DeloistFeedback = (() => {
  let api = null;
  let toast = () => {};
  let kind = 'bug';
  let sending = false;

  const el = (id) => document.getElementById(id);

  function markKind() {
    document.querySelectorAll('#fb-kind [data-value]').forEach((btn) => {
      btn.setAttribute('aria-checked', String(btn.dataset.value === kind));
    });
    el('fb-text').placeholder = kind === 'bug'
      ? tr('Что случилось и как должно было быть')
      : tr('Чего не хватает и как это могло бы работать');
  }

  function open() {
    el('fb-error').hidden = true;
    markKind();
    el('modal-feedback').showModal();
    el('fb-text').focus();
  }

  async function send(event) {
    event.preventDefault();
    if (sending) return;
    const text = el('fb-text').value.trim();
    const error = el('fb-error');
    if (!text) {
      error.textContent = tr('Напиши, что случилось или чего не хватает');
      error.hidden = false;
      el('fb-text').focus();
      return;
    }
    sending = true;
    error.hidden = true;
    const button = el('fb-send');
    button.disabled = true;
    button.textContent = tr('Отправляю…');
    try {
      await api('/api/feedback', { method: 'POST', body: { kind, text, contact: el('fb-contact').value.trim() } });
      el('fb-text').value = '';
      el('fb-contact').value = '';
      el('modal-feedback').close();
      toast(tr('Спасибо! Сообщение отправлено'), 4000);
    } catch (err) {
      // Написанное остаётся в окне: человек не должен набирать его заново.
      error.textContent = trServer(err.message);
      error.hidden = false;
    } finally {
      sending = false;
      button.disabled = false;
      button.textContent = tr('Отправить');
    }
  }

  function start(deps) {
    api = deps.api;
    toast = deps.toast;
    el('btn-feedback').addEventListener('click', open);
    el('form-feedback').addEventListener('submit', send);
    document.querySelectorAll('#fb-kind [data-value]').forEach((btn) => {
      btn.addEventListener('click', () => {
        kind = btn.dataset.value;
        markKind();
      });
    });
  }

  return { start };
})();
