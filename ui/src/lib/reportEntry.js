// Вход на экран «Отчёт»: кнопка #btn-report в боковой панели открывает окно #modal-report.
// Компонент монтируется заново при каждом открытии, чтобы цифры были свежими.
export function bindReportEntry(getUi, doc = typeof document === 'undefined' ? null : document) {
  if (!doc) return;
  const wire = () => {
    const btn = doc.getElementById('btn-report');
    const dlg = doc.getElementById('modal-report');
    const host = doc.getElementById('report-host');
    if (!btn || !dlg || !host) return;
    let view = null;
    const drop = () => {
      view?.destroy();
      view = null;
      host.replaceChildren();
    };
    btn.addEventListener('click', () => {
      const ui = getUi();
      if (!ui?.mountReport || typeof dlg.showModal !== 'function') return;
      drop();
      dlg.showModal();
      view = ui.mountReport(host, {});
    });
    doc.getElementById('report-close')?.addEventListener('click', () => dlg.close());
    // «close» приходит позже самого Esc: если окно уже открыли заново, свежий экран не трогаем.
    dlg.addEventListener('close', () => {
      if (!dlg.open) drop();
    });
  };
  if (doc.readyState === 'loading') doc.addEventListener('DOMContentLoaded', wire, { once: true });
  else wire();
}
