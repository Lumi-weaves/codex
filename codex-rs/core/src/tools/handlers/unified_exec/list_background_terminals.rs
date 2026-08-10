use crate::codex_thread::BackgroundTerminalInfo;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::shell_spec::create_list_background_terminals_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Serialize;

pub struct ListBackgroundTerminalsHandler;

impl ToolExecutor<ToolInvocation> for ListBackgroundTerminalsHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("list_background_terminals")
    }

    fn spec(&self) -> ToolSpec {
        create_list_background_terminals_tool()
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        // Read-only snapshot of the process manager; safe to run in parallel.
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(self.handle_call(invocation))
    }
}

impl ListBackgroundTerminalsHandler {
    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;
        if !matches!(payload, ToolPayload::Function { .. }) {
            return Err(FunctionCallError::RespondToModel(
                "list_background_terminals handler received unsupported payload".to_string(),
            ));
        }

        let terminals = session.list_background_terminals().await;
        let result = ListBackgroundTerminalsResult::try_from_terminals(&terminals)?;
        let content = serde_json::to_string(&result).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize list_background_terminals response: {err}"
            ))
        })?;

        Ok(boxed_tool_output(FunctionToolOutput::from_text(
            content,
            Some(true),
        )))
    }
}

impl CoreToolRuntime for ListBackgroundTerminalsHandler {}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub(crate) struct BackgroundTerminalEntry {
    session_id: i32,
    item_id: String,
    command: String,
    cwd: String,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub(crate) struct ListBackgroundTerminalsResult {
    terminals: Vec<BackgroundTerminalEntry>,
}

impl ListBackgroundTerminalsResult {
    pub(crate) fn try_from_terminals(
        terminals: &[BackgroundTerminalInfo],
    ) -> Result<Self, FunctionCallError> {
        let terminals = terminals
            .iter()
            .map(|terminal| {
                let session_id = terminal.process_id.parse().map_err(|err| {
                    FunctionCallError::Fatal(format!(
                        "background terminal has invalid process id {:?}: {err}",
                        terminal.process_id
                    ))
                })?;
                Ok(BackgroundTerminalEntry {
                    session_id,
                    item_id: terminal.item_id.clone(),
                    command: terminal.command.clone(),
                    cwd: terminal.cwd.inferred_native_path_string(),
                })
            })
            .collect::<Result<Vec<_>, FunctionCallError>>()?;
        Ok(Self { terminals })
    }
}
