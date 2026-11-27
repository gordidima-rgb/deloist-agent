// Цепочка процесса списком: шаги в порядке линий (без заметок и фигур). Чистая функция, без DOM.
import { isAnnotation } from './workflows/graph.js';

/** graph: { nodes, edges } → узлы-шаги: сначала те, у кого нет входящих линий, дальше по линиям; ровные ветки — слева направо. */
export function orderSteps(graph) {
  const nodes = (graph?.nodes ?? []).filter((n) => !isAnnotation(n.kind));
  const ids = new Set(nodes.map((n) => n.id));
  const edges = (graph?.edges ?? []).filter((e) => ids.has(e.from) && ids.has(e.to));
  const left = new Map(nodes.map((n) => [n.id, edges.filter((e) => e.to === n.id).length]));
  const place = (a, b) => a.x - b.x || a.y - b.y;
  const ready = nodes.filter((n) => !left.get(n.id)).sort(place);
  const out = [];
  while (ready.length) {
    const node = ready.shift();
    out.push(node);
    for (const e of edges.filter((x) => x.from === node.id)) {
      left.set(e.to, left.get(e.to) - 1);
      if (!left.get(e.to)) {
        ready.push(nodes.find((n) => n.id === e.to));
        ready.sort(place);
      }
    }
  }
  // Цикл или разорванная линия: оставшиеся шаги не теряем.
  return [...out, ...nodes.filter((n) => !out.includes(n)).sort(place)];
}
