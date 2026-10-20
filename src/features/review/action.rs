//! Action-mode routing for installed review tools.

use super::*;

/// Action mode: parse context and route to setup, a direct review pane, or the
/// picker according to the installed tools on `PATH`.
pub(crate) fn run_action() -> Result<(), String> {
    let json = env_non_blank("HERDR_PLUGIN_CONTEXT_JSON").ok_or_else(|| {
        "HERDR_PLUGIN_CONTEXT_JSON is not set; invoke this action from herdr".to_string()
    })?;
    let ctx = context::Context::parse(&json)?;
    let identity = CallerIdentity::resolve(
        &ctx,
        env_non_blank("HERDR_WORKSPACE_ID").as_deref(),
        env_non_blank("HERDR_PANE_ID").as_deref(),
        env_non_blank("HERDR_TAB_ID").as_deref(),
    );
    let path = std::env::var("PATH").unwrap_or_default();
    let deps = ActionDeps {
        load_config: &settings::load,
        open: &|config, dir, tool_id, workspace, caller_pane, caller_agent, caller_tab| {
            herdr::open_review_pane(
                config,
                workspace,
                dir,
                tool_id,
                caller_pane,
                caller_agent,
                caller_tab,
            )
        },
        open_setup: &herdr::open_setup_popup,
        focus: &herdr::tab_focus,
        log: &|msg| eprintln!("{msg}"),
        warn: &|msg| eprintln!("herdr-review: warning: {msg}"),
    };
    run_action_with(&identity, &ctx.dir, &path, &deps)
}

/// Caller identity resolved from context with environment fallbacks.
#[cfg_attr(test, derive(Debug))]
pub(super) struct CallerIdentity {
    pub(super) workspace: Option<String>,
    pub(super) pane: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) tab: Option<String>,
}

impl CallerIdentity {
    pub(super) fn resolve(
        ctx: &context::Context,
        workspace_env: Option<&str>,
        pane_env: Option<&str>,
        tab_env: Option<&str>,
    ) -> Self {
        Self {
            workspace: ctx
                .workspace_id
                .clone()
                .or_else(|| workspace_env.map(str::to_string)),
            pane: ctx
                .focused_pane_id
                .clone()
                .or_else(|| pane_env.map(str::to_string)),
            agent: ctx.focused_pane_agent.clone(),
            tab: ctx.tab_id.clone().or_else(|| tab_env.map(str::to_string)),
        }
    }
}

type OpenFn<'a> = &'a dyn for<'c> Fn(
    &'c settings::Config,
    &str,
    &str,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    Option<&str>,
) -> Result<Option<herdr::PaneIds>, String>;

pub(super) struct ActionDeps<'a> {
    pub(super) load_config: &'a dyn Fn() -> settings::Config,
    pub(super) open: OpenFn<'a>,
    pub(super) open_setup: &'a dyn Fn() -> Result<(), String>,
    pub(super) focus: &'a dyn Fn(&str) -> Result<(), String>,
    pub(super) log: &'a dyn Fn(&str),
    pub(super) warn: &'a dyn Fn(&str),
}

impl ReviewActionHost for ActionDeps<'_> {
    fn load_config(&self) -> settings::Config {
        (self.load_config)()
    }

    fn open_review(&self, request: OpenReview<'_>) -> Result<Option<herdr::PaneIds>, String> {
        (self.open)(
            request.config,
            request.dir,
            request.tool_id,
            request.workspace,
            request.caller_pane,
            request.caller_agent,
            request.caller_tab,
        )
    }

    fn open_setup(&self) -> Result<(), String> {
        (self.open_setup)()
    }

    fn open_tool_picker<'a>(&self, request: OpenToolPicker<'a>) -> Result<(), String> {
        herdr::open_tool_picker(&request)
    }

    fn focus(&self, tab_id: &str) -> Result<(), String> {
        (self.focus)(tab_id)
    }

    fn log(&self, message: &str) {
        (self.log)(message);
    }

    fn warn(&self, message: &str) {
        (self.warn)(message);
    }
}

pub(super) fn run_action_with(
    identity: &CallerIdentity,
    dir: &str,
    path: &str,
    deps: &dyn ReviewActionHost,
) -> Result<(), String> {
    ReviewAction {
        identity,
        dir,
        path,
        deps,
    }
    .run()
}

struct ReviewAction<'a> {
    identity: &'a CallerIdentity,
    dir: &'a str,
    path: &'a str,
    deps: &'a dyn ReviewActionHost,
}

impl ReviewAction<'_> {
    fn run(&self) -> Result<(), String> {
        let identity = self.identity;
        let dir = self.dir;
        let path = self.path;
        let deps = self.deps;
        let config = deps.load_config();
        let installed = tool::installed_ids(path);
        if installed.is_empty() {
            deps.open_setup()?;
            deps.log("herdr-review: opened review tool setup");
            return Ok(());
        }
        if installed.len() > 1 {
            deps.open_tool_picker(OpenToolPicker {
                tool_ids: &installed,
                config: &config,
                dir,
                workspace: identity.workspace.as_deref(),
                caller_pane: identity.pane.as_deref(),
                caller_agent: identity.agent.as_deref(),
                caller_tab: identity.tab.as_deref(),
            })?;
            deps.log("herdr-review: opened review tool picker");
            return Ok(());
        }
        let tool = ReviewTool::from_id(installed[0])
            .ok_or_else(|| format!("unsupported installed review tool id: {}", installed[0]))?;
        let mut config = config;
        if matches!(
            config.placement,
            settings::Placement::Split | settings::Placement::Zoomed
        ) && identity.pane.is_none()
        {
            deps.warn(&format!(
                "placement \"{}\" needs a caller pane; using \"tab\"",
                config.placement.as_str()
            ));
            config.placement = settings::Placement::Tab;
        }
        let ids = deps.open_review(OpenReview {
            config: &config,
            dir,
            tool_id: tool.id(),
            workspace: identity.workspace.as_deref(),
            caller_pane: identity.pane.as_deref(),
            caller_agent: identity.agent.as_deref(),
            caller_tab: identity.tab.as_deref(),
        })?;
        if let Some(ids) = ids {
            match &ids.tab_id {
                Some(tab) => {
                    deps.log(&format!(
                        "herdr-review: opened review pane {} in tab {}",
                        ids.pane_id, tab
                    ));
                    let _ = deps.focus(tab);
                }
                None => deps.log(&format!("herdr-review: opened review pane {}", ids.pane_id)),
            }
        }
        Ok(())
    }
}
