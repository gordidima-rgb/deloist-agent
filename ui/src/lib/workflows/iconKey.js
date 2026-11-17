// Имя значка рабочих процессов -> имя в реестре lib/icons.js (общее для WfIcon и меню холста).
const SHARED = { plus: 'plus', graph: 'workflow', check: 'check', x: 'close', link2: 'link', file: 'file', front: 'wf-dup' };

export function wfIconKey(name) {
  return SHARED[name] ?? `wf-${String(name).replaceAll('_', '-')}`;
}
