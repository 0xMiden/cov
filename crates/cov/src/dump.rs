//! JSON rendering of a raw run file, for inspection and debugging.

use miden_cov_collector::{RunFile, RunOutcome};
use serde_json::{Value, json};

/// Renders a run with its raw hits, before any source mapping.
///
/// Shape: `{name, outcome, cycles, packages: [{commitment, node_starts,
/// op_hits, edge_starts}]}` where `outcome` is `"ok"` or `{"failed":
/// message}`, `node_starts` is a list of `[node, count]`, `op_hits` a list
/// of `[node, op, count]`, and `edge_starts` a list of `[parent, child,
/// count]`.
pub fn run_to_json(run: &RunFile) -> Value {
    let outcome = match &run.outcome {
        RunOutcome::Ok => json!("ok"),
        RunOutcome::Failed(message) => json!({ "failed": message }),
    };
    let packages: Vec<Value> = run
        .packages
        .iter()
        .map(|(commitment, hits)| {
            json!({
                "commitment": commitment.to_hex(),
                "node_starts": hits
                    .node_starts
                    .iter()
                    .map(|(node, count)| json!([node, count]))
                    .collect::<Vec<_>>(),
                "op_hits": hits
                    .op_hits
                    .iter()
                    .map(|((node, op), count)| json!([node, op, count]))
                    .collect::<Vec<_>>(),
                "edge_starts": hits
                    .edge_starts
                    .iter()
                    .map(|((parent, child), count)| json!([parent, child, count]))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "name": run.name,
        "outcome": outcome,
        "cycles": run.cycles,
        "packages": packages,
    })
}

#[cfg(test)]
mod tests {
    use miden_core::Word;
    use miden_cov_collector::PackageHits;

    use super::*;

    #[test]
    fn renders_outcome_and_hits() {
        let mut hits = PackageHits::default();
        hits.record_node_start(3);
        hits.record_op(3, 1);
        hits.record_op(3, 1);
        hits.record_edge_start(2, 3);
        let run = RunFile {
            name: "t".into(),
            outcome: RunOutcome::Failed("boom".into()),
            cycles: 9,
            packages: vec![(Word::default(), hits)],
        };
        assert_eq!(
            run_to_json(&run),
            json!({
                "name": "t",
                "outcome": { "failed": "boom" },
                "cycles": 9,
                "packages": [{
                    "commitment": Word::default().to_hex(),
                    "node_starts": [[3, 1]],
                    "op_hits": [[3, 1, 2]],
                    "edge_starts": [[2, 3, 1]],
                }],
            })
        );
        let run = RunFile {
            outcome: RunOutcome::Ok,
            packages: vec![],
            ..run
        };
        assert_eq!(run_to_json(&run)["outcome"], json!("ok"));
    }
}
