//! The stripping logic: remove everything Jupyter regenerates on execution —
//! cell outputs, execution counts, transient UI metadata — and nothing an
//! author wrote. Modeled on `deshaw/nbstripout-fast`'s `stripoutlib.rs`
//! (itself a port of `kynan/nbstripout`), trimmed to this repo's needs.

use serde_json::{Map, Value};

type JsonMap = Map<String, Value>;

// nbformat-4 keys (see https://nbformat.readthedocs.io/en/latest/format_description.html).
const CELLS: &str = "cells";
const METADATA: &str = "metadata";
const OUTPUTS: &str = "outputs";
const EXECUTION_COUNT: &str = "execution_count";
const TAGS: &str = "tags";

/// The opt-out marker: outputs survive when the notebook's metadata, a cell's
/// metadata, or a cell's tags carry `keep_output` (the `nbstripout`
/// convention). An explicit `keep_output: false` in metadata beats a tag.
const KEEP_OUTPUT: &str = "keep_output";

/// Notebook-level metadata that only records UI/session state.
const NOTEBOOK_METADATA_STRIP: [&str; 3] = ["signature", "vscode", "widgets"];
/// Cell-level metadata that only records UI/session state.
const CELL_METADATA_STRIP: [&str; 6] = [
    "ExecuteTime",
    "collapsed",
    "execution",
    "heading_collapsed",
    "hidden",
    "scrolled",
];

/// Strip the notebook in place. Idempotent; leaves non-notebook JSON alone
/// (no `cells` array means there is nothing to do).
pub fn strip(nb: &mut Value) {
    if !nb.get(CELLS).is_some_and(Value::is_array) {
        return;
    }
    let keep_all = nb
        .get(METADATA)
        .and_then(|m| m.get(KEEP_OUTPUT))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if let Some(meta) = nb.get_mut(METADATA).and_then(Value::as_object_mut) {
        for key in NOTEBOOK_METADATA_STRIP {
            meta.remove(key);
        }
    }

    if let Some(cells) = nb.get_mut(CELLS).and_then(Value::as_array_mut) {
        for cell in cells.iter_mut().filter_map(Value::as_object_mut) {
            strip_cell(cell, keep_all);
        }
    }
}

fn strip_cell(cell: &mut JsonMap, keep_all: bool) {
    let keep = keep_all || cell_keeps_output(cell);
    if let Some(outputs) = cell.get_mut(OUTPUTS).and_then(Value::as_array_mut) {
        if keep {
            // Kept outputs still lose their counts (an `execute_result`
            // carries one), so re-runs don't churn the diff.
            for output in outputs.iter_mut().filter_map(Value::as_object_mut) {
                null_key(output, EXECUTION_COUNT);
            }
        } else {
            outputs.clear();
        }
    }
    // Nulled, not removed: the nbformat schema requires the key on code cells.
    null_key(cell, EXECUTION_COUNT);
    if let Some(meta) = cell.get_mut(METADATA).and_then(Value::as_object_mut) {
        for key in CELL_METADATA_STRIP {
            meta.remove(key);
        }
    }
}

fn cell_keeps_output(cell: &JsonMap) -> bool {
    let Some(meta) = cell.get(METADATA).and_then(Value::as_object) else {
        return false;
    };
    meta.get(KEEP_OUTPUT)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            meta.get(TAGS)
                .and_then(Value::as_array)
                .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some(KEEP_OUTPUT)))
        })
}

fn null_key(obj: &mut JsonMap, key: &str) {
    if obj.contains_key(key) {
        obj.insert(key.to_owned(), Value::Null);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[allow(clippy::needless_pass_by_value)] // json!-consuming helpers read better by value
mod tests {
    use super::strip;
    use serde_json::{json, Value};

    fn code_cell(outputs: Value, metadata: Value) -> Value {
        json!({
            "cell_type": "code",
            "execution_count": 7,
            "id": "abcd1234",
            "metadata": metadata,
            "outputs": outputs,
            "source": ["print('hi')"],
        })
    }

    fn some_output() -> Value {
        json!([{
            "data": {"text/plain": ["42"]},
            "execution_count": 7,
            "metadata": {},
            "output_type": "execute_result",
        }])
    }

    fn nb(cells: Value) -> Value {
        json!({
            "cells": cells,
            "metadata": {"kernelspec": {"name": "python3"}, "widgets": {"x": 1}},
            "nbformat": 4,
            "nbformat_minor": 5,
        })
    }

    #[test]
    fn strips_outputs_counts_and_transient_metadata() {
        let mut v = nb(json!([code_cell(
            some_output(),
            json!({"scrolled": true, "editable": false})
        )]));
        strip(&mut v);
        let cell = &v["cells"][0];
        assert_eq!(cell["outputs"], json!([]));
        assert_eq!(cell["execution_count"], Value::Null);
        // Transient key gone, authored key intact.
        assert_eq!(cell["metadata"], json!({"editable": false}));
        // Notebook-level: widgets gone, kernelspec intact.
        assert_eq!(v["metadata"], json!({"kernelspec": {"name": "python3"}}));
    }

    #[test]
    fn keep_output_tag_keeps_outputs_but_not_counts() {
        let mut v = nb(json!([code_cell(
            some_output(),
            json!({"tags": ["keep_output"]})
        )]));
        strip(&mut v);
        let cell = &v["cells"][0];
        assert_eq!(cell["outputs"].as_array().map(Vec::len), Some(1));
        assert_eq!(cell["outputs"][0]["execution_count"], Value::Null);
        assert_eq!(cell["execution_count"], Value::Null);
    }

    #[test]
    fn explicit_metadata_false_beats_the_tag() {
        let mut v = nb(json!([code_cell(
            some_output(),
            json!({"keep_output": false, "tags": ["keep_output"]})
        )]));
        strip(&mut v);
        assert_eq!(v["cells"][0]["outputs"], json!([]));
    }

    #[test]
    fn notebook_level_keep_output_keeps_everything() {
        let mut v = nb(json!([code_cell(some_output(), json!({}))]));
        v["metadata"]["keep_output"] = json!(true);
        strip(&mut v);
        assert_eq!(v["cells"][0]["outputs"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn markdown_cells_pass_through_untouched() {
        let md = json!({
            "cell_type": "markdown",
            "id": "md1",
            "metadata": {},
            "source": ["# title"],
        });
        let mut v = nb(json!([md]));
        strip(&mut v);
        assert_eq!(v["cells"][0], md);
    }

    #[test]
    fn idempotent() {
        let mut once = nb(json!([code_cell(some_output(), json!({"scrolled": true}))]));
        strip(&mut once);
        let mut twice = once.clone();
        strip(&mut twice);
        assert_eq!(once, twice);
    }

    #[test]
    fn non_notebook_json_is_left_alone() {
        let orig = json!({"metadata": {"widgets": 1}});
        let mut v = orig.clone();
        strip(&mut v);
        assert_eq!(v, orig);
        let mut plain = json!([1, 2, 3]);
        strip(&mut plain);
        assert_eq!(plain, json!([1, 2, 3]));
    }
}
