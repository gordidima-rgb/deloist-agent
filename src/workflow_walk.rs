//! Обход графа рабочего процесса: чистые функции без ввода-вывода.
//! По графу и уже сделанным шагам говорят, какие узлы пора запустить, а какие пропустить.
//! Правила — `notes/workflows-spec.md`, раздел «Движок».

use std::collections::{HashMap, HashSet};

use crate::workflows::{is_annotation, Edge, Graph, Step};

/// Сколько раз один узел может сработать в запуске.
pub const MAX_PASSES: usize = 100;
/// Сколько шагов может быть в запуске.
pub const MAX_STEPS: usize = 500;
/// Потолок текста входа.
pub const MAX_INPUT_CHARS: usize = 32_000;
const FAILED_CHARS: usize = 300;

pub fn terminal(status: &str) -> bool {
    matches!(status, "done" | "failed" | "skipped" | "stopped")
}

/// Что отдельно решил запуск: какие запускающие узлы молчат и откуда идёт «запустить отсюда».
#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub skip: HashSet<String>,
    pub forced: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    /// Откуда пришёл вход.
    pub from: String,
    pub step_id: String,
    /// Что течёт по связи: text, image или file.
    pub ty: String,
    /// Предыдущий шаг упал: во вход пойдёт текст об ошибке, а не ответ.
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Start { node: String, inputs: Vec<Input> },
    Skip { node: String },
}

#[derive(Debug, Default)]
pub struct Next {
    pub actions: Vec<Action>,
    /// Слишком много кругов: запуск пора валить.
    pub overflow: bool,
}

fn runnable(graph: &Graph) -> impl Iterator<Item = &crate::workflows::Node> {
    graph.nodes.iter().filter(|n| !is_annotation(&n.kind))
}

/// Какой план у запуска. Ручной запуск — только ручные запускающие узлы (а если их нет, все);
/// запуск по расписанию — только сработавший узел; «запустить отсюда» — всё, что не ниже этого узла, молчит.
pub fn plan(graph: &Graph, from_node: Option<&str>, trigger_node: Option<&str>, solo: bool) -> Plan {
    let mut plan = Plan::default();
    if let Some(from) = from_node {
        let mut reach: HashSet<&str> = HashSet::from([from]);
        let mut grew = true;
        while grew {
            grew = false;
            for edge in &graph.edges {
                if reach.contains(edge.from.as_str()) && reach.insert(&edge.to) {
                    grew = true;
                }
            }
        }
        plan.skip = runnable(graph).filter(|n| !reach.contains(n.id.as_str()) || (solo && n.id != from)).map(|n| n.id.clone()).collect();
        plan.forced = Some(from.to_string());
        return plan;
    }
    let triggers: Vec<_> = graph.nodes.iter().filter(|n| n.kind == "trigger").collect();
    if let Some(fired) = trigger_node {
        plan.skip = triggers.iter().filter(|n| n.id != fired).map(|n| n.id.clone()).collect();
        return plan;
    }
    let is_manual = |n: &crate::workflows::Node| matches!(n.text("trigger").as_str(), "" | "manual");
    if triggers.iter().any(|n| is_manual(n)) {
        plan.skip = triggers.iter().filter(|n| !is_manual(n)).map(|n| n.id.clone()).collect();
    }
    plan
}

/// Обратные рёбра: те, что замыкают петлю. Обход в глубину от узлов без входов, потом от оставшихся, в порядке узлов.
pub fn back_edges(graph: &Graph) -> HashSet<String> {
    let ids: HashSet<&str> = runnable(graph).map(|n| n.id.as_str()).collect();
    let edges: Vec<&Edge> = graph.edges.iter().filter(|e| ids.contains(e.from.as_str()) && ids.contains(e.to.as_str())).collect();
    let has_input: HashSet<&str> = edges.iter().map(|e| e.to.as_str()).collect();
    let mut order: Vec<&str> = runnable(graph).filter(|n| !has_input.contains(n.id.as_str())).map(|n| n.id.as_str()).collect();
    order.extend(runnable(graph).filter(|n| has_input.contains(n.id.as_str())).map(|n| n.id.as_str()));
    let mut out: HashMap<&str, Vec<&Edge>> = HashMap::new();
    for edge in &edges {
        out.entry(edge.from.as_str()).or_default().push(edge);
    }
    // 0 — не был, 1 — на стеке, 2 — закончен.
    let mut color: HashMap<&str, u8> = HashMap::new();
    let mut back = HashSet::new();
    for start in order {
        if color.contains_key(start) {
            continue;
        }
        color.insert(start, 1);
        // Явный стек: (узел, номер следующего ребра).
        let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
        while let Some((node, index)) = stack.pop() {
            let list = out.get(node).map_or(&[][..], Vec::as_slice);
            if index >= list.len() {
                color.insert(node, 2);
                continue;
            }
            stack.push((node, index + 1));
            let edge = list[index];
            match color.get(edge.to.as_str()) {
                Some(1) => {
                    back.insert(edge.id.clone());
                }
                Some(_) => {}
                None => {
                    color.insert(edge.to.as_str(), 1);
                    stack.push((edge.to.as_str(), 0));
                }
            }
        }
    }
    back
}

/// Активно ли ребро: источник закончил успешно, а у условия ещё и совпала ветка.
fn edge_active(edge: &Edge, source: &Step) -> bool {
    source.status == "done" && edge.branch.as_ref().is_none_or(|b| source.branch.as_deref() == Some(b.as_str()))
}

fn collect_inputs(graph: &Graph, incoming: &[&Edge], latest: &HashMap<&str, &Step>) -> (Vec<Input>, usize) {
    let mut inputs = Vec::new();
    let mut active = 0;
    for edge in incoming {
        let Some(source) = latest.get(edge.from.as_str()) else { continue };
        if edge_active(edge, source) {
            inputs.push(Input { from: edge.from.clone(), step_id: source.id.clone(), ty: crate::workflows::edge_type_of(graph, edge), failed: false });
            active += 1;
        } else if source.status == "failed" {
            inputs.push(Input { from: edge.from.clone(), step_id: source.id.clone(), ty: "text".into(), failed: true });
        }
    }
    (inputs, active)
}

/// Входы узла по самым свежим шагам его предков (для повтора шага).
pub fn inputs_of(graph: &Graph, steps: &[Step], node_id: &str) -> Vec<Input> {
    let mut latest: HashMap<&str, &Step> = HashMap::new();
    for step in steps {
        if latest.get(step.node_id.as_str()).is_none_or(|old| old.seq < step.seq) {
            latest.insert(step.node_id.as_str(), step);
        }
    }
    let incoming: Vec<&Edge> = graph.edges.iter().filter(|e| e.to == node_id).collect();
    collect_inputs(graph, &incoming, &latest).0
}

/// Что делать дальше. Узел стартует, когда все его прямые предки закончились; берётся «хватит одного активного входа».
pub fn next(graph: &Graph, steps: &[Step], plan: &Plan) -> Next {
    let back = back_edges(graph);
    let mut latest: HashMap<&str, &Step> = HashMap::new();
    let mut passes: HashMap<&str, usize> = HashMap::new();
    for step in steps {
        *passes.entry(step.node_id.as_str()).or_default() += 1;
        if latest.get(step.node_id.as_str()).is_none_or(|old| old.seq < step.seq) {
            latest.insert(step.node_id.as_str(), step);
        }
    }
    let ids: HashSet<&str> = runnable(graph).map(|n| n.id.as_str()).collect();
    let mut result = Next::default();
    for node in runnable(graph) {
        let last = latest.get(node.id.as_str()).copied();
        if last.is_some_and(|l| !terminal(&l.status)) {
            continue;
        }
        let incoming: Vec<&Edge> = graph.edges.iter().filter(|e| e.to == node.id && ids.contains(e.from.as_str())).collect();
        let forward: Vec<&&Edge> = incoming.iter().filter(|e| !back.contains(&e.id)).collect();
        let ready = forward.iter().all(|e| latest.get(e.from.as_str()).is_some_and(|s| terminal(&s.status)));
        if !ready {
            continue;
        }
        let due = match last {
            None => true,
            Some(l) => {
                let fresh_forward = forward.iter().any(|e| latest.get(e.from.as_str()).is_some_and(|s| (edge_active(e, s) || s.status == "failed") && s.end_seq > l.seq));
                let fresh_back = incoming
                    .iter()
                    .filter(|e| back.contains(&e.id))
                    .any(|e| latest.get(e.from.as_str()).is_some_and(|s| edge_active(e, s) && s.end_seq > l.seq));
                fresh_forward || fresh_back
            }
        };
        if !due {
            continue;
        }
        if passes.get(node.id.as_str()).copied().unwrap_or(0) >= MAX_PASSES || steps.len() + result.actions.len() >= MAX_STEPS {
            result.overflow = true;
            result.actions.clear();
            return result;
        }
        if last.is_none() && plan.skip.contains(&node.id) {
            result.actions.push(Action::Skip { node: node.id.clone() });
            continue;
        }
        let (inputs, active) = collect_inputs(graph, &incoming, &latest);
        let forced_start = last.is_none() && plan.forced.as_deref() == Some(node.id.as_str());
        if active == 0 {
            if incoming.is_empty() || forced_start {
                result.actions.push(Action::Start { node: node.id.clone(), inputs: Vec::new() });
            } else {
                result.actions.push(Action::Skip { node: node.id.clone() });
            }
            continue;
        }
        result.actions.push(Action::Start { node: node.id.clone(), inputs });
    }
    result
}

/// Чем кончился запуск, когда шагов в работе больше нет: остановленный шаг — `stopped`, упавший — `failed`, иначе `done`.
pub fn outcome(graph: &Graph, steps: &[Step]) -> &'static str {
    let mut latest: HashMap<&str, &Step> = HashMap::new();
    for step in steps {
        if latest.get(step.node_id.as_str()).is_none_or(|old| old.seq < step.seq) {
            latest.insert(step.node_id.as_str(), step);
        }
    }
    let known: HashSet<&str> = runnable(graph).map(|n| n.id.as_str()).collect();
    let states: Vec<&str> = latest.iter().filter(|(id, _)| known.contains(**id)).map(|(_, s)| s.status.as_str()).collect();
    if states.contains(&"stopped") {
        "stopped"
    } else if states.contains(&"failed") {
        "failed"
    } else {
        "done"
    }
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
}

/// Текст об упавшей ветке для следующего шага.
pub fn failed_text(error: &str) -> String {
    let flat = error.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("Нет ответа: шаг упал. Ошибка: {}", clip(&flat, FAILED_CHARS))
}

/// Склеить входы: один — как есть, несколько — «Вход 1:…». Потолок 32 000 символов.
pub fn join_inputs(parts: &[String]) -> String {
    let text = match parts {
        [] => String::new(),
        [one] => one.clone(),
        many => many.iter().enumerate().map(|(i, p)| format!("Вход {}:\n\n{p}", i + 1)).collect::<Vec<_>>().join("\n\n"),
    };
    clip(&text, MAX_INPUT_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::Node;

    fn node(id: &str, kind: &str) -> Node {
        Node { id: id.into(), kind: kind.into(), ..Node::default() }
    }

    fn edge(from: &str, to: &str, branch: Option<&str>) -> Edge {
        Edge { id: format!("{from}>{to}{}", branch.unwrap_or("")), from: from.into(), to: to.into(), edge_type: None, branch: branch.map(Into::into) }
    }

    fn graph(nodes: &[(&str, &str)], edges: Vec<Edge>) -> Graph {
        Graph { nodes: nodes.iter().map(|(id, kind)| node(id, kind)).collect(), edges, strokes: vec![] }
    }

    /// Простой исполнитель: каждый шаг кончается сразу. `verdict` решает судьбу узла: ok, fail или ветка условия.
    struct Sim {
        steps: Vec<Step>,
        counter: u64,
    }

    impl Sim {
        fn new() -> Self {
            Self { steps: Vec::new(), counter: 0 }
        }

        fn push(&mut self, node: &str, status: &str, branch: Option<&str>) {
            self.counter += 1;
            let seq = self.counter;
            self.counter += 1;
            self.steps.push(Step {
                id: format!("s{seq}"), node_id: node.into(), status: status.into(), branch: branch.map(Into::into), seq, end_seq: self.counter,
                output: format!("out-{node}"), ..Step::default()
            });
        }

        fn run(&mut self, g: &Graph, plan: &Plan, verdict: impl Fn(&str, usize) -> &'static str) -> Option<&'static str> {
            for _ in 0..2000 {
                let n = next(g, &self.steps, plan);
                if n.overflow {
                    return None;
                }
                if n.actions.is_empty() {
                    return Some(outcome(g, &self.steps));
                }
                for action in n.actions {
                    match action {
                        Action::Skip { node } => self.push(&node, "skipped", None),
                        Action::Start { node, .. } => {
                            let count = self.steps.iter().filter(|s| s.node_id == node).count();
                            match verdict(&node, count) {
                                "fail" => self.push(&node, "failed", None),
                                "yes" => self.push(&node, "done", Some("yes")),
                                "no" => self.push(&node, "done", Some("no")),
                                _ => self.push(&node, "done", None),
                            }
                        }
                    }
                }
            }
            None
        }

        fn status(&self, node: &str) -> Vec<&str> {
            self.steps.iter().filter(|s| s.node_id == node).map(|s| s.status.as_str()).collect()
        }
    }

    #[test]
    fn parallel_branches_start_together_and_the_join_waits_for_all() {
        let g = graph(&[("p", "prompt"), ("a", "bot"), ("b", "bot"), ("j", "bot")], vec![edge("p", "a", None), edge("p", "b", None), edge("a", "j", None), edge("b", "j", None)]);
        let mut sim = Sim::new();
        sim.push("p", "done", None);
        let first = next(&g, &sim.steps, &Plan::default());
        let started: Vec<_> = first.actions.iter().map(|a| matches!(a, Action::Start { node, .. } if node == "a" || node == "b")).collect();
        assert_eq!(started, vec![true, true], "обе ветки сразу");
        sim.push("a", "done", None);
        assert!(next(&g, &sim.steps, &Plan::default()).actions.iter().all(|a| !matches!(a, Action::Start { node, .. } if node == "j")), "соединение ждёт вторую ветку");
        sim.push("b", "done", None);
        let after = next(&g, &sim.steps, &Plan::default());
        match &after.actions[..] {
            [Action::Start { node, inputs }] => {
                assert_eq!(node, "j");
                assert_eq!(inputs.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn join_with_one_failed_branch_still_runs_with_the_failure_text() {
        let g = graph(&[("p", "prompt"), ("a", "bot"), ("b", "bot"), ("j", "bot")], vec![edge("p", "a", None), edge("p", "b", None), edge("a", "j", None), edge("b", "j", None)]);
        let mut sim = Sim::new();
        let result = sim.run(&g, &Plan::default(), |node, _| if node == "a" { "fail" } else { "ok" });
        assert_eq!(sim.status("j"), vec!["done"]);
        assert_eq!(result, Some("failed"), "упавший шаг виден в итоге запуска");
        let mut at_join = Sim::new();
        at_join.push("p", "done", None);
        at_join.push("a", "failed", None);
        at_join.push("b", "done", None);
        let Action::Start { inputs, .. } = &next(&g, &at_join.steps, &Plan::default()).actions[0] else { panic!() };
        assert_eq!(inputs.iter().map(|i| i.failed).collect::<Vec<_>>(), vec![true, false]);
    }

    #[test]
    fn a_node_whose_every_input_failed_is_skipped() {
        let g = graph(&[("p", "prompt"), ("a", "bot"), ("b", "bot")], vec![edge("p", "a", None), edge("a", "b", None)]);
        let mut sim = Sim::new();
        sim.run(&g, &Plan::default(), |node, _| if node == "a" { "fail" } else { "ok" });
        assert_eq!(sim.status("b"), vec!["skipped"]);
    }

    #[test]
    fn condition_routes_to_the_matching_branch_and_skips_the_other() {
        let g = graph(
            &[("p", "prompt"), ("c", "condition"), ("y", "output"), ("n", "output"), ("after", "output")],
            vec![edge("p", "c", None), edge("c", "y", Some("yes")), edge("c", "n", Some("no")), edge("n", "after", None)],
        );
        let mut yes = Sim::new();
        yes.run(&g, &Plan::default(), |node, _| if node == "c" { "yes" } else { "ok" });
        assert_eq!(yes.status("y"), vec!["done"]);
        assert_eq!(yes.status("n"), vec!["skipped"]);
        assert_eq!(yes.status("after"), vec!["skipped"], "пропуск идёт дальше по ветке");
        let mut no = Sim::new();
        no.run(&g, &Plan::default(), |node, _| if node == "c" { "no" } else { "ok" });
        assert_eq!(no.status("y"), vec!["skipped"]);
        assert_eq!(no.status("n"), vec!["done"]);
        assert_eq!(no.status("after"), vec!["done"]);
    }

    #[test]
    fn a_loop_runs_until_the_condition_says_no_more() {
        // p -> work -> check; check(yes) -> work (обратное ребро), check(no) -> out.
        let g = graph(
            &[("p", "prompt"), ("work", "bot"), ("check", "condition"), ("out", "output")],
            vec![edge("p", "work", None), edge("work", "check", None), edge("check", "work", Some("yes")), edge("check", "out", Some("no"))],
        );
        assert_eq!(back_edges(&g), HashSet::from(["check>workyes".to_string()]));
        let mut sim = Sim::new();
        // Условие «да» два раза, потом «нет».
        let result = sim.run(&g, &Plan::default(), |node, count| if node == "check" { if count < 2 { "yes" } else { "no" } } else { "ok" });
        assert_eq!(result, Some("done"));
        assert_eq!(sim.status("work").len(), 3);
        assert_eq!(sim.status("check").len(), 3);
        // Пока условие говорило «да», выход честно пропускался; когда сказало «нет» — сработал.
        assert_eq!(sim.status("out"), vec!["skipped", "done"]);
    }

    #[test]
    fn an_endless_loop_hits_the_pass_cap() {
        let g = graph(
            &[("p", "prompt"), ("work", "bot"), ("check", "condition"), ("out", "output")],
            vec![edge("p", "work", None), edge("work", "check", None), edge("check", "work", Some("yes")), edge("check", "out", Some("no"))],
        );
        let mut sim = Sim::new();
        let result = sim.run(&g, &Plan::default(), |node, _| if node == "check" { "yes" } else { "ok" });
        assert_eq!(result, None, "слишком много кругов");
        assert!(sim.status("work").len() <= MAX_PASSES);
        assert!(sim.steps.len() <= MAX_STEPS);
    }

    #[test]
    fn plans_decide_which_triggers_fire() {
        let mut g = graph(&[("m", "trigger"), ("s", "trigger"), ("a", "bot")], vec![edge("m", "a", None), edge("s", "a", None)]);
        g.nodes[0].fields.insert("trigger".into(), "manual".into());
        g.nodes[1].fields.insert("trigger".into(), "schedule".into());
        assert_eq!(plan(&g, None, None, false).skip, HashSet::from(["s".to_string()]));
        assert_eq!(plan(&g, None, Some("s"), false).skip, HashSet::from(["m".to_string()]));
        // Нет ручных узлов: «Запустить» поднимает всё.
        g.nodes[0].fields.insert("trigger".into(), "schedule".into());
        assert!(plan(&g, None, None, false).skip.is_empty());
        let mut sim = Sim::new();
        sim.run(&g, &plan(&g, None, Some("s"), false), |_, _| "ok");
        assert_eq!(sim.status("m"), vec!["skipped"]);
        assert_eq!(sim.status("a"), vec!["done"], "одного активного входа хватает");
    }

    #[test]
    fn run_from_a_node_skips_everything_not_below_it_and_feeds_it_empty_input() {
        let g = graph(&[("p", "prompt"), ("a", "bot"), ("b", "bot")], vec![edge("p", "a", None), edge("a", "b", None)]);
        let mut sim = Sim::new();
        sim.run(&g, &plan(&g, Some("a"), None, false), |_, _| "ok");
        assert_eq!(sim.status("p"), vec!["skipped"]);
        assert_eq!(sim.status("a"), vec!["done"]);
        assert_eq!(sim.status("b"), vec!["done"]);
    }

    #[test]
    fn a_retried_step_makes_its_followers_run_again() {
        let g = graph(&[("p", "prompt"), ("a", "bot"), ("b", "bot")], vec![edge("p", "a", None), edge("a", "b", None)]);
        let mut sim = Sim::new();
        sim.run(&g, &Plan::default(), |node, _| if node == "a" { "fail" } else { "ok" });
        assert_eq!(sim.status("b"), vec!["skipped"]);
        sim.push("a", "done", None);
        let again = next(&g, &sim.steps, &Plan::default());
        assert!(matches!(&again.actions[..], [Action::Start { node, .. }] if node == "b"));
    }

    #[test]
    fn inputs_are_joined_and_failures_are_short() {
        assert_eq!(join_inputs(&["один".into()]), "один");
        assert_eq!(join_inputs(&["а".into(), "б".into()]), "Вход 1:\n\nа\n\nВход 2:\n\nб");
        assert!(failed_text(&"x".repeat(1000)).chars().count() < 360);
        assert_eq!(join_inputs(&["я".repeat(40_000)]).chars().count(), MAX_INPUT_CHARS + 1);
    }
}
