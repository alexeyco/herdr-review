//! Parsing the herdr plugin context JSON.

use serde::Deserialize;
use serde_json::Value;

/// Values read from `HERDR_PLUGIN_CONTEXT_JSON` that the review action needs.
#[derive(Debug)]
pub(crate) struct Context {
    /// Review directory: `worktree.checkout_path` → `workspace_cwd` → `focused_pane_cwd`.
    pub(crate) dir: String,
    /// `workspace_id`, when a non-blank string.
    pub(crate) workspace_id: Option<String>,
    /// `focused_pane_id`, when a non-blank string.
    pub(crate) focused_pane_id: Option<String>,
    /// `focused_pane_agent`, when a non-blank string.
    pub(crate) focused_pane_agent: Option<String>,
    /// `tab_id` — the focused pane's tab, when a non-blank string.
    pub(crate) tab_id: Option<String>,
}

impl Context {
    /// Parses the plugin context and extracts the fields used by the action.
    pub(crate) fn parse(json: &str) -> Result<Self, String> {
        let raw: RawContext =
            serde_json::from_str(json).map_err(|e| format!("invalid plugin context JSON: {e}"))?;
        let dir = raw.dir().ok_or_else(|| {
            "no directory in plugin context; invoke this action inside a workspace".to_string()
        })?;
        Ok(Self {
            dir,
            workspace_id: raw.workspace_id.as_ref().and_then(value_string),
            focused_pane_id: raw.focused_pane_id.as_ref().and_then(value_string),
            focused_pane_agent: raw.focused_pane_agent.as_ref().and_then(value_string),
            tab_id: raw.tab_id.as_ref().and_then(value_string),
        })
    }
}

#[derive(Deserialize)]
struct RawContext {
    worktree: Option<Worktree>,
    workspace_cwd: Option<Value>,
    focused_pane_cwd: Option<Value>,
    workspace_id: Option<Value>,
    focused_pane_id: Option<Value>,
    focused_pane_agent: Option<Value>,
    tab_id: Option<Value>,
}

#[derive(Deserialize)]
struct Worktree {
    checkout_path: Option<Value>,
}

impl RawContext {
    fn dir(&self) -> Option<String> {
        self.worktree
            .as_ref()
            .and_then(|worktree| worktree.checkout_path.as_ref())
            .and_then(value_string)
            .or_else(|| self.workspace_cwd.as_ref().and_then(value_string))
            .or_else(|| self.focused_pane_cwd.as_ref().and_then(value_string))
    }
}

fn value_string(value: &Value) -> Option<String> {
    value.as_str().and_then(non_blank).map(str::to_string)
}

fn non_blank(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Context, String> {
        Context::parse(json)
    }

    const CTX: &str = concat!(
        r#"{"workspace_id":"ws-1","workspace_label":"demo","tab_id":"t1","#,
        r#""worktree":{"repo_key":"github.com/u/demo","repo_name":"demo","#,
        r#""repo_root":"/home/u/demo","checkout_path":"/home/u/demo","is_linked_worktree":false},"#,
        r#""workspace_cwd":"/home/u/demo/src","focused_pane_id":"p1","#,
        r#""focused_pane_agent":"claude","focused_pane_cwd":"/home/u/demo/src/tests"}"#
    );

    #[test]
    fn full_context_prefers_checkout_path() {
        let ctx = parse(CTX).unwrap();
        assert_eq!(ctx.dir, "/home/u/demo");
        assert_eq!(ctx.workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(ctx.focused_pane_id.as_deref(), Some("p1"));
        assert_eq!(ctx.focused_pane_agent.as_deref(), Some("claude"));
        assert_eq!(ctx.tab_id.as_deref(), Some("t1"));
    }

    #[test]
    fn falls_back_to_workspace_cwd() {
        let json = r#"{"worktree":{},"workspace_cwd":"/home/u/demo/src"}"#;
        assert_eq!(parse(json).unwrap().dir, "/home/u/demo/src");
    }

    #[test]
    fn falls_back_to_focused_pane_cwd() {
        let json = r#"{"workspace_cwd":"  ","focused_pane_cwd":"/home/u/demo/src"}"#;
        assert_eq!(parse(json).unwrap().dir, "/home/u/demo/src");
    }

    #[test]
    fn non_string_dir_is_absent() {
        for json in [r#"{"workspace_cwd":42}"#, r#"{"workspace_cwd":null}"#] {
            assert!(parse(json).unwrap_err().contains("no directory"));
        }
    }

    #[test]
    fn empty_string_value_falls_through_to_next_candidate() {
        let json = r#"{"worktree":{"checkout_path":"  "},"workspace_cwd":"/home/u/demo"}"#;
        assert_eq!(parse(json).unwrap().dir, "/home/u/demo");
    }

    #[test]
    fn all_blank_values_are_an_error() {
        for json in [
            r#"{"workspace_cwd":""}"#,
            r#"{"worktree":{"checkout_path":" "},"workspace_cwd":"","focused_pane_cwd":"\t"}"#,
        ] {
            assert!(parse(json).unwrap_err().contains("no directory"));
        }
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse("{not json").unwrap_err().contains("invalid"));
    }

    #[test]
    fn optional_fields_missing_are_none() {
        let ctx = parse(r#"{"workspace_cwd":"/repo"}"#).unwrap();
        assert!(ctx.workspace_id.is_none());
        assert!(ctx.focused_pane_id.is_none());
        assert!(ctx.focused_pane_agent.is_none());
        assert!(ctx.tab_id.is_none());
    }

    #[test]
    fn optional_fields_blank_or_non_string_are_none() {
        let json = r#"{"workspace_cwd":"/repo","workspace_id":" ","focused_pane_id":7,"focused_pane_agent":null,"tab_id":"  "}"#;
        let ctx = parse(json).unwrap();
        assert!(ctx.workspace_id.is_none());
        assert!(ctx.focused_pane_id.is_none());
        assert!(ctx.focused_pane_agent.is_none());
        assert!(ctx.tab_id.is_none());
    }

    #[test]
    fn non_string_tab_id_is_absent() {
        // A `tab_id` that is not a string counts as absent; the caller-tab
        // pair is then omitted from the review-pane open.
        for json in [
            r#"{"workspace_cwd":"/repo","tab_id":42}"#,
            r#"{"workspace_cwd":"/repo","tab_id":null}"#,
        ] {
            assert!(parse(json).unwrap().tab_id.is_none(), "{json}");
        }
    }

    #[test]
    fn unescapes_windows_and_unicode_paths() {
        let json = r#"{"worktree":{"checkout_path":"C:\\Users\\caf\u00e9"}}"#;
        assert_eq!(parse(json).unwrap().dir, r"C:\Users\café");
    }
}
