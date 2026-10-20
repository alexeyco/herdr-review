use crate::adapters::tool::ReviewTool;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Deliver,
    NoAnnotations,
    NoAgent,
    Failed(i32),
}

pub(super) fn decide(
    tool: ReviewTool,
    exit_code: i32,
    has_annotations: bool,
    has_agent: bool,
) -> Outcome {
    let success = exit_code == 0 || (tool == ReviewTool::Revdiff && exit_code == 10);
    if !success {
        return Outcome::Failed(exit_code);
    }
    if has_annotations {
        if has_agent {
            Outcome::Deliver
        } else {
            Outcome::NoAgent
        }
    } else {
        Outcome::NoAnnotations
    }
}

pub(super) fn prompt_text(tool: ReviewTool, output: &str) -> String {
    format!(
        "Review annotations from {}:\n\n{}",
        tool.id(),
        output.trim()
    )
}
