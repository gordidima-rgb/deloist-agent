//! Граф файлов воркспейса бота, как graphify, но без модели: файлы, заголовки документов и объявления
//! в коде — узлы; «содержит», ссылки и импорты, упоминания файлов и использования объявлений — связи.
//!
//! Бот не читает папку целиком: `workspace_graph` находит нужное по словам (`query`), показывает,
//! кто на что ссылается (`explain`), и как связаны две части (`path`). Граф пересобирается, только
//! когда в папке что-то поменялось, и лежит вне воркспейса, чтобы файловые инструменты его не видели.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::App;

const MAX_FILES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 1_000_000;
const SKIP_DIRS: [&str; 9] = [".git", "node_modules", "target", "dist", "build", ".venv", "__pycache__", ".next", ".cache"];
const TEXT_EXTS: [&str; 30] = [
    "md", "txt", "csv", "tsv", "json", "yaml", "yml", "toml", "ini", "html", "htm", "xml", "css", "js", "mjs", "ts", "tsx",
    "jsx", "py", "rs", "go", "gd", "java", "kt", "cs", "php", "rb", "sh", "ps1", "sql",
];
/// Объявления короче не считаем: `id`, `run`, `get` встречаются везде и дают ложные связи.
const MIN_SYMBOL: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    /// `file`, `heading` или `symbol`.
    pub kind: String,
    pub name: String,
    pub file: String,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// `contains`, `links`, `imports`, `mentions`, `uses`.
    pub kind: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Graph {
    pub stamp: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

fn ext(path: &str) -> String {
    Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase()
}

fn symbol_patterns(ext: &str) -> Vec<Regex> {
    let list: &[&str] = match ext {
        "rs" => &[r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|mod|type|const|static)\s+([A-Za-z_]\w*)"],
        "py" => &[r"^\s*(?:async\s+)?(?:def|class)\s+([A-Za-z_]\w*)"],
        "js" | "mjs" | "ts" | "tsx" | "jsx" => &[
            r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?(?:function\*?|class)\s+([A-Za-z_$][\w$]*)",
            r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:\(|function|[A-Za-z_$][\w$]*\s*=>)",
        ],
        "go" => &[r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)", r"^type\s+([A-Za-z_]\w*)"],
        "gd" => &[r"^\s*(?:static\s+)?func\s+([A-Za-z_]\w*)", r"^\s*class_name\s+([A-Za-z_]\w*)"],
        "java" | "kt" | "cs" => &[r"^\s*(?:public|private|protected|internal|static|final|abstract|open|data|\s)*(?:class|interface|enum|fun|record)\s+([A-Za-z_]\w*)"],
        "php" => &[r"^\s*(?:public|private|protected|static|\s)*(?:function|class)\s+([A-Za-z_]\w*)"],
        "rb" => &[r"^\s*(?:def|class|module)\s+(?:self\.)?([A-Za-z_]\w*)"],
        "sql" => &[r"(?i)^\s*create\s+(?:or\s+replace\s+)?(?:table|view|function|procedure)\s+(?:if\s+not\s+exists\s+)?([A-Za-z_][\w.]*)"],
        _ => &[],
    };
    list.iter().filter_map(|p| Regex::new(p).ok()).collect()
}

/// Путь ссылки относительно файла: `docs/a.md` + `../b.md` → `b.md`. Наружу из воркспейса не выходим.
fn resolve(from: &str, target: &str) -> Option<String> {
    let target = target.split(['#', '?']).next()?.trim();
    if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
        return None;
    }
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { from.split('/').collect() };
    if !target.starts_with('/') {
        parts.pop();
    }
    for piece in target.trim_start_matches('/').split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn identifiers(text: &str) -> HashMap<String, usize> {
    let mut first: HashMap<String, usize> = HashMap::new();
    for (n, line) in text.lines().enumerate() {
        let mut word = String::new();
        for c in line.chars().chain(std::iter::once(' ')) {
            if c.is_alphanumeric() || c == '_' || c == '$' {
                word.push(c);
            } else if !word.is_empty() {
                first.entry(std::mem::take(&mut word)).or_insert(n + 1);
            }
        }
    }
    first
}

/// Построить граф по файлам: (путь относительно воркспейса с `/`, текст).
pub fn build(files: &[(String, String)]) -> Graph {
    let mut graph = Graph::default();
    let mut file_index: HashMap<String, usize> = HashMap::new();
    for (path, _) in files {
        file_index.insert(path.clone(), graph.nodes.len());
        graph.nodes.push(Node { kind: "file".into(), name: path.clone(), file: path.clone(), line: 0 });
    }
    let heading = Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").unwrap();
    let md_link = Regex::new(r"\]\(\s*<?([^)\s>]+)>?").unwrap();
    let js_import = Regex::new(r#"(?:from\s+|import\s*\(\s*|require\s*\(\s*)['"](\.{1,2}/[^'"]+)['"]"#).unwrap();
    let mut symbols: HashMap<String, Vec<usize>> = HashMap::new();
    let mut edges: HashSet<(usize, usize, String)> = HashSet::new();
    let mut add_edge = |graph: &mut Graph, from: usize, to: usize, kind: &str, line: usize| {
        if from != to && edges.insert((from, to, kind.to_string())) {
            graph.edges.push(Edge { from, to, kind: kind.into(), line });
        }
    };

    for (path, text) in files {
        let file = file_index[path];
        let kind = ext(path);
        let patterns = symbol_patterns(&kind);
        for (n, line) in text.lines().enumerate() {
            if kind == "md" {
                if let Some(c) = heading.captures(line) {
                    let id = graph.nodes.len();
                    graph.nodes.push(Node { kind: "heading".into(), name: c[2].to_string(), file: path.clone(), line: n + 1 });
                    add_edge(&mut graph, file, id, "contains", n + 1);
                }
            }
            for pattern in &patterns {
                if let Some(c) = pattern.captures(line) {
                    let name = c[1].to_string();
                    if name.chars().count() >= MIN_SYMBOL {
                        let id = graph.nodes.len();
                        graph.nodes.push(Node { kind: "symbol".into(), name: name.clone(), file: path.clone(), line: n + 1 });
                        add_edge(&mut graph, file, id, "contains", n + 1);
                        symbols.entry(name).or_default().push(id);
                    }
                    break;
                }
            }
            let mut targets: Vec<(String, &str)> = md_link.captures_iter(line).map(|c| (c[1].to_string(), "links")).collect();
            targets.extend(js_import.captures_iter(line).map(|c| (c[1].to_string(), "imports")));
            for (target, edge) in targets {
                let Some(resolved) = resolve(path, &target) else { continue };
                let found = [resolved.clone(), format!("{resolved}.js"), format!("{resolved}.ts"), format!("{resolved}/index.js"), format!("{resolved}.md")]
                    .into_iter()
                    .find_map(|candidate| file_index.get(&candidate).copied());
                if let Some(to) = found {
                    add_edge(&mut graph, file, to, edge, n + 1);
                }
            }
        }
    }

    // Упоминание имени другого файла и использование объявлений из других файлов.
    let basenames: std::collections::BTreeMap<String, usize> = {
        let mut count: HashMap<String, Vec<usize>> = HashMap::new();
        for (path, id) in &file_index {
            let base = path.rsplit('/').next().unwrap_or(path).to_lowercase();
            if base.contains('.') && base.chars().count() >= 5 {
                count.entry(base).or_default().push(*id);
            }
        }
        // Одинаковые имена в разных папках неоднозначны: такие упоминания не связываем.
        count.into_iter().filter(|(_, ids)| ids.len() == 1).map(|(base, ids)| (base, ids[0])).collect()
    };
    for (path, text) in files {
        let file = file_index[path];
        let lower = text.to_lowercase();
        for (base, to) in &basenames {
            let linked = graph.edges.iter().any(|e| e.from == file && e.to == *to);
            if *to != file && !linked && lower.contains(base.as_str()) {
                let line = lower.lines().position(|l| l.contains(base.as_str())).map_or(0, |n| n + 1);
                add_edge(&mut graph, file, *to, "mentions", line);
            }
        }
        let words = identifiers(text);
        let mut names: Vec<(&String, &Vec<usize>)> = symbols.iter().collect();
        names.sort();
        for (name, ids) in names {
            // Имя объявлено в нескольких местах — неясно, какое использовано: связываем, только если одно.
            if ids.len() != 1 {
                continue;
            }
            let symbol = ids[0];
            if graph.nodes[symbol].file == *path {
                continue;
            }
            if let Some(line) = words.get(name) {
                add_edge(&mut graph, file, symbol, "uses", *line);
            }
        }
    }
    graph
}

fn label(node: &Node) -> String {
    match node.kind.as_str() {
        "file" => format!("файл {}", node.file),
        "heading" => format!("раздел «{}» — {}:{}", node.name, node.file, node.line),
        _ => format!("{} — {}:{}", node.name, node.file, node.line),
    }
}

fn edge_word(kind: &str, outgoing: bool) -> &'static str {
    match (kind, outgoing) {
        ("contains", true) => "содержит",
        ("contains", false) => "лежит в",
        ("links", true) => "ссылается на",
        ("links", false) => "на него ссылается",
        ("imports", true) => "импортирует",
        ("imports", false) => "импортируется из",
        ("mentions", true) => "упоминает",
        ("mentions", false) => "упоминается в",
        ("uses", true) => "использует",
        _ => "используется в",
    }
}

fn stem(word: &str) -> String {
    let count = word.chars().count();
    let keep = if count > 5 { count - 2 } else { count };
    word.chars().take(keep).collect()
}

fn score(node: &Node, stems: &[String]) -> usize {
    let hay = format!("{} {}", node.name, node.file).to_lowercase();
    stems.iter().filter(|s| hay.contains(s.as_str())).count()
}

/// Узел по имени: точное совпадение имени или пути, иначе лучшее по словам.
fn find(graph: &Graph, wanted: &str) -> Option<usize> {
    let lower = wanted.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }
    if let Some(i) = graph.nodes.iter().position(|n| n.name.to_lowercase() == lower || n.file.to_lowercase() == lower) {
        return Some(i);
    }
    let stems: Vec<String> = lower.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.chars().count() >= 2).map(stem).collect();
    (0..graph.nodes.len()).map(|i| (i, score(&graph.nodes[i], &stems))).filter(|(_, s)| *s > 0).max_by_key(|(i, s)| (*s, usize::MAX - i)).map(|(i, _)| i)
}

fn degree(graph: &Graph, id: usize) -> usize {
    graph.edges.iter().filter(|e| e.from == id || e.to == id).count()
}

pub fn query(graph: &Graph, text: &str) -> String {
    let stems: Vec<String> = text.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.chars().count() >= 2).map(stem).collect();
    let mut hits: Vec<(usize, usize)> = (0..graph.nodes.len()).map(|i| (i, score(&graph.nodes[i], &stems))).filter(|(_, s)| *s > 0).collect();
    hits.sort_by_key(|(i, s)| (usize::MAX - s, usize::MAX - degree(graph, *i)));
    if hits.is_empty() {
        return format!("В графе файлов по «{}» ничего нет.", text.trim());
    }
    let lines: Vec<String> = hits.iter().take(15).map(|(i, _)| format!("{} (связей: {})", label(&graph.nodes[*i]), degree(graph, *i))).collect();
    format!("Нашлось {}:\n{}\nПодробнее — action explain с именем узла.", hits.len(), lines.join("\n"))
}

pub fn explain(graph: &Graph, name: &str) -> String {
    let Some(id) = find(graph, name) else {
        return format!("Узла «{}» в графе файлов нет.", name.trim());
    };
    let mut out = vec![label(&graph.nodes[id])];
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for edge in &graph.edges {
        let (other, outgoing) = if edge.from == id {
            (edge.to, true)
        } else if edge.to == id {
            (edge.from, false)
        } else {
            continue;
        };
        let word = edge_word(&edge.kind, outgoing).to_string();
        let where_ = if outgoing && edge.line > 0 { format!(" (строка {})", edge.line) } else if !outgoing && edge.line > 0 { format!(" — {}:{}", graph.nodes[edge.from].file, edge.line) } else { String::new() };
        let item = format!("{}{where_}", label(&graph.nodes[other]));
        match groups.iter_mut().find(|(w, _)| *w == word) {
            Some((_, items)) => items.push(item),
            None => groups.push((word, vec![item])),
        }
    }
    if groups.is_empty() {
        out.push("Связей нет.".into());
    }
    for (word, items) in groups {
        let more = if items.len() > 15 { format!("\n  …и ещё {}", items.len() - 15) } else { String::new() };
        out.push(format!("{word}:\n  {}{more}", items.into_iter().take(15).collect::<Vec<_>>().join("\n  ")));
    }
    out.join("\n")
}

pub fn path(graph: &Graph, a: &str, b: &str) -> String {
    let (Some(start), Some(goal)) = (find(graph, a), find(graph, b)) else {
        return "Одного из узлов нет в графе файлов.".into();
    };
    let mut adjacent: HashMap<usize, Vec<(usize, usize, bool)>> = HashMap::new();
    for (n, edge) in graph.edges.iter().enumerate() {
        adjacent.entry(edge.from).or_default().push((edge.to, n, true));
        adjacent.entry(edge.to).or_default().push((edge.from, n, false));
    }
    let mut came: HashMap<usize, (usize, usize, bool)> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    let mut seen = HashSet::from([start]);
    while let Some(at) = queue.pop_front() {
        if at == goal {
            break;
        }
        for &(next, edge, outgoing) in adjacent.get(&at).into_iter().flatten() {
            if seen.insert(next) {
                came.insert(next, (at, edge, outgoing));
                queue.push_back(next);
            }
        }
    }
    if !seen.contains(&goal) {
        return format!("{} и {} не связаны.", label(&graph.nodes[start]), label(&graph.nodes[goal]));
    }
    let mut steps = Vec::new();
    let mut at = goal;
    while let Some(&(prev, edge, outgoing)) = came.get(&at) {
        steps.push(format!("  —{}→ {}", edge_word(&graph.edges[edge].kind, outgoing), label(&graph.nodes[at])));
        at = prev;
    }
    steps.reverse();
    format!("{}\n{}", label(&graph.nodes[start]), steps.join("\n"))
}

// ---------------------------------------------------------------------------
// Сборка по папке бота
// ---------------------------------------------------------------------------

fn collect(root: &Path) -> Vec<(String, PathBuf, u64, i64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push(path);
                }
                continue;
            }
            let rel = path.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if rel.is_empty() || !TEXT_EXTS.contains(&ext(&rel).as_str()) || meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
            out.push((rel, path, meta.len(), modified));
            if out.len() >= MAX_FILES {
                return out;
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn cache_path(app: &App, bot_id: &str) -> PathBuf {
    app.store.data_dir().join("graphs").join(format!("{bot_id}.json"))
}

/// Граф папки бота: из кэша, если в папке ничего не поменялось, иначе собирается заново.
pub async fn for_bot(app: &Arc<App>, bot_id: &str) -> Graph {
    let root = app.store.workspace(bot_id);
    let cache = cache_path(app, bot_id);
    let built = tokio::task::spawn_blocking(move || {
        use std::hash::{Hash, Hasher};
        let files = collect(&root);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (rel, _, len, modified) in &files {
            (rel, len, modified).hash(&mut hasher);
        }
        let stamp = format!("{:x}", hasher.finish());
        if let Ok(text) = std::fs::read_to_string(&cache) {
            if let Ok(graph) = serde_json::from_str::<Graph>(&text) {
                if graph.stamp == stamp {
                    return graph;
                }
            }
        }
        let texts: Vec<(String, String)> = files
            .iter()
            .filter_map(|(rel, path, _, _)| std::fs::read(path).ok().map(|bytes| (rel.clone(), String::from_utf8_lossy(&bytes).into_owned())))
            .collect();
        let mut graph = build(&texts);
        graph.stamp = stamp;
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cache, serde_json::to_string(&graph).unwrap_or_default());
        graph
    })
    .await;
    built.unwrap_or_default()
}

pub fn tool_def() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "workspace_graph",
            "description": "Граф файлов твоего воркспейса: файлы, разделы документов и объявления в коде, связанные ссылками, \
                импортами, упоминаниями и использованиями. Быстрее, чем читать файлы подряд. query — найти по словам; \
                explain — что содержит узел, на что ссылается и кто ссылается на него, с файлом и строкой; path — как связаны a и b.",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["query", "explain", "path"] },
                    "q": { "type": "string", "description": "Слова для query или имя узла для explain: файл, раздел, функция." },
                    "a": { "type": "string", "description": "Первый узел для path." },
                    "b": { "type": "string", "description": "Второй узел для path." },
                },
                "required": ["action"],
            },
        },
    })
}

pub async fn exec(app: &Arc<App>, bot_id: &str, args: &Value) -> Result<String, String> {
    let text = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let graph = for_bot(app, bot_id).await;
    if graph.nodes.is_empty() {
        return Ok("В воркспейсе нет текстовых файлов: граф пуст.".into());
    }
    let summary = format!("Граф: файлов {}, узлов {}, связей {}.\n", graph.nodes.iter().filter(|n| n.kind == "file").count(), graph.nodes.len(), graph.edges.len());
    let body = match text("action").as_str() {
        "explain" => explain(&graph, &text("q")),
        "path" => path(&graph, &text("a"), &text("b")),
        "query" | "" => query(&graph, &text("q")),
        other => return Err(format!("неизвестное действие: {other}")),
    };
    Ok(crate::tools::clamp(&format!("{summary}{body}"), 6_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<(String, String)> {
        vec![
            ("README.md".into(), "# Отчёты\nСм. [план](docs/plan.md) и скрипт report.py\n## Как запускать\n".into()),
            ("docs/plan.md".into(), "# План продаж\nДанные берёт [выгрузка](../data/sales.csv)\n".into()),
            ("data/sales.csv".into(), "date,sum\n2026-11-01,100\n".into()),
            ("report.py".into(), "import csv\n\ndef load_sales(path):\n    return list(csv.reader(open(path)))\n\ndef build_report():\n    rows = load_sales('data/sales.csv')\n".into()),
            ("web/app.js".into(), "import { fmt } from './util.js'\nexport function renderReport(rows) { return fmt(rows) }\n".into()),
            ("web/util.js".into(), "export const formatRows = (rows) => rows.join(',')\nexport function fmt(x) { return formatRows(x) }\n".into()),
        ]
    }

    #[test]
    fn files_headings_symbols_and_links_become_a_graph() {
        let graph = build(&files());
        let names: Vec<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
        for wanted in ["README.md", "Как запускать", "load_sales", "build_report", "renderReport", "formatRows"] {
            assert!(names.contains(&wanted), "{wanted}: {names:?}");
        }
        assert!(!names.contains(&"fmt"), "короткие имена не узлы");
        let explained = explain(&graph, "load_sales");
        assert!(explained.starts_with("load_sales — report.py:3"), "{explained}");
        let readme = explain(&graph, "README.md");
        assert!(readme.contains("ссылается на:\n  файл docs/plan.md (строка 2)"), "{readme}");
        assert!(readme.contains("упоминает:\n  файл report.py"), "{readme}");
        let app = explain(&graph, "web/app.js");
        assert!(app.contains("импортирует:\n  файл web/util.js"), "{app}");
        let util = explain(&graph, "formatRows");
        assert!(util.contains("лежит в"), "{util}");
    }

    #[test]
    fn query_and_path_walk_the_graph() {
        let graph = build(&files());
        let found = query(&graph, "продаж sales");
        assert!(found.contains("load_sales") && found.contains("файл data/sales.csv"), "{found}");
        let route = path(&graph, "README.md", "data/sales.csv");
        assert!(route.starts_with("файл README.md\n"), "{route}");
        assert!(route.contains("файл data/sales.csv"), "{route}");
        assert!(path(&graph, "README.md", "нет такого").contains("нет в графе"));
    }

    #[test]
    fn links_stay_inside_the_workspace() {
        assert_eq!(resolve("docs/a.md", "../b.md").as_deref(), Some("b.md"));
        assert_eq!(resolve("a.md", "../../etc/passwd"), None);
        assert_eq!(resolve("a.md", "https://x.ru/a.md"), None);
        assert_eq!(resolve("docs/a.md", "./c.md#part").as_deref(), Some("docs/c.md"));
    }
}
