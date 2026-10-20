use std::path::Path;

use crate::adapters::git::Scope;
use crate::adapters::tool::ReviewTool;
use crate::settings::Config;

use super::process::ChildResult;

#[cfg_attr(test, mockall::automock)]
pub(crate) trait ReviewRepository {
    fn scope(&self) -> Scope;
}

#[cfg_attr(test, mockall::automock)]
pub(crate) trait ReviewToolRunner {
    fn run(&self, tool: ReviewTool, argv: &[String], dir: &Path) -> Result<ChildResult, String>;
}

#[cfg_attr(test, mockall::automock)]
pub(crate) trait ReviewActionHost {
    fn load_config(&self) -> Config;
    fn open_review<'a>(
        &self,
        request: OpenReview<'a>,
    ) -> Result<Option<crate::adapters::herdr::PaneIds>, String>;
    fn open_tool_picker<'a>(&self, request: OpenToolPicker<'a>) -> Result<(), String>;
    fn open_setup(&self) -> Result<(), String>;
    fn focus(&self, tab_id: &str) -> Result<(), String>;
    fn log(&self, message: &str);
    fn warn(&self, message: &str);
}

pub(crate) struct OpenToolPicker<'a> {
    pub(crate) tool_ids: &'a [&'static str],
    pub(crate) config: &'a Config,
    pub(crate) dir: &'a str,
    pub(crate) workspace: Option<&'a str>,
    pub(crate) caller_pane: Option<&'a str>,
    pub(crate) caller_agent: Option<&'a str>,
    pub(crate) caller_tab: Option<&'a str>,
}

pub(crate) struct OpenReview<'a> {
    pub(crate) config: &'a Config,
    pub(crate) dir: &'a str,
    pub(crate) tool_id: &'a str,
    pub(crate) workspace: Option<&'a str>,
    pub(crate) caller_pane: Option<&'a str>,
    pub(crate) caller_agent: Option<&'a str>,
    pub(crate) caller_tab: Option<&'a str>,
}

#[cfg_attr(test, mockall::automock)]
pub(crate) trait ReviewDeliveryHost {
    fn deliver(&self, pane_id: &str, prompt: &str) -> Result<(), String>;
    fn close(&self);
    fn focus(&self, tab_id: &str) -> Result<(), String>;
    fn warn(&self, message: &str);
    fn hold(&self);
}
