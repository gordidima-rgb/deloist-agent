// Рисование на холсте: штрихи в координатах потока ([[x,y],…]). Без DOM и без Svelte.
export const MAX_STROKES = 500;
export const MAX_POINTS = 2000;

// Выбросить точки ближе `min` к предыдущей: штрих остаётся плавным, но лёгким.
export function thin(points, min = 1.5) {
  const out = [];
  for (const p of points) {
    const last = out[out.length - 1];
    if (!last || Math.hypot(p[0] - last[0], p[1] - last[1]) >= min) out.push([Math.round(p[0] * 10) / 10, Math.round(p[1] * 10) / 10]);
  }
  return out;
}

// Не больше MAX_POINTS: равномерно прореживаем, концы оставляем.
export function limitPoints(points, max = MAX_POINTS) {
  if (points.length <= max) return points;
  const step = (points.length - 1) / (max - 1);
  const out = [];
  for (let i = 0; i < max; i += 1) out.push(points[Math.round(i * step)]);
  return out;
}

// Путь SVG: квадратичные кривые через середины отрезков. Одна точка — точка (кружок штриха).
export function pathOf(points) {
  if (!points.length) return '';
  if (points.length === 1) {
    const [x, y] = points[0];
    return `M${x} ${y}l0.01 0`;
  }
  if (points.length === 2) return `M${points[0][0]} ${points[0][1]}L${points[1][0]} ${points[1][1]}`;
  let d = `M${points[0][0]} ${points[0][1]}`;
  for (let i = 1; i < points.length - 1; i += 1) {
    const mx = (points[i][0] + points[i + 1][0]) / 2;
    const my = (points[i][1] + points[i + 1][1]) / 2;
    d += `Q${points[i][0]} ${points[i][1]} ${mx} ${my}`;
  }
  const last = points[points.length - 1];
  return d + `L${last[0]} ${last[1]}`;
}

function distToSegment(px, py, ax, ay, bx, by) {
  const dx = bx - ax;
  const dy = by - ay;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / len2));
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
}

// Попал ли ластик (круг радиуса r в точке) в штрих; толщина штриха прибавляется к радиусу.
export function hitsStroke(stroke, x, y, r) {
  const pts = stroke.points || [];
  const reach = r + (stroke.width || 0) / 2;
  if (pts.length === 1) return Math.hypot(x - pts[0][0], y - pts[0][1]) <= reach;
  for (let i = 0; i < pts.length - 1; i += 1) {
    if (distToSegment(x, y, pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1]) <= reach) return true;
  }
  return false;
}
