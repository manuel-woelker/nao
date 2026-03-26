use crate::run_engine::RunEngine;
use nao_base::err;
use nao_base::result::NaoResult;
use nao_recipe::Task;

pub(super) fn split_task_selectors(task_names: &[String]) -> Vec<String> {
    task_names
        .iter()
        .flat_map(|task_name| task_name.split(','))
        .map(str::trim)
        .filter(|task_name| !task_name.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn expand_task_selectors(
    tasks: &[Task],
    selectors: &[String],
) -> NaoResult<Vec<String>> {
    let mut expanded = Vec::new();

    for selector in selectors {
        if selector.contains('_') {
            expanded.extend(expand_wildcard_selector(tasks, selector)?);
        } else {
            expanded.push(selector.clone());
        }
    }

    Ok(expanded)
}

/* 📖 # Why do task specifiers use `_` as the wildcard instead of `*`?
Using `*` would force callers to quote task specifiers in most shells because the shell expands
asterisks before `nao` sees the argument. `_` keeps wildcard task selection available from the
command line without extra quoting or platform-specific escaping rules.
*/
fn expand_wildcard_selector(tasks: &[Task], selector: &str) -> NaoResult<Vec<String>> {
    let pattern_parts = selector.split('_').collect::<Vec<_>>();
    let matches = tasks
        .iter()
        .filter(|task| task_name_matches_selector(task.name.as_str(), &pattern_parts))
        .map(|task| task.name.as_str().to_owned())
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return Err(err!("task specifier `{selector}` did not match any tasks"));
    }

    Ok(matches)
}

fn task_name_matches_selector(task_name: &str, pattern_parts: &[&str]) -> bool {
    let mut remaining = task_name;

    for (index, part) in pattern_parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if index == 0 {
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
            continue;
        }

        match remaining.find(part) {
            Some(position) => {
                remaining = &remaining[position + part.len()..];
            }
            None => return false,
        }
    }

    selector_has_trailing_wildcard(pattern_parts) || remaining.is_empty()
}

fn selector_has_trailing_wildcard(pattern_parts: &[&str]) -> bool {
    pattern_parts.last().is_some_and(|part| part.is_empty())
}

#[allow(dead_code)]
fn _keep_module_referenced(_: &RunEngine) {}
