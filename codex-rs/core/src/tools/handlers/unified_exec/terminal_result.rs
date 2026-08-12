use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::shell_spec::create_read_terminal_result_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;

const DEFAULT_TERMINAL_RESULT_READ_MAX_BYTES: usize = 4096;
const MAX_TERMINAL_RESULT_READ_MAX_BYTES: usize = 6000;

#[derive(Debug, Deserialize)]
struct ReadTerminalResultArgs {
    result_ref: String,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    max_bytes: Option<usize>,
}

pub struct ReadTerminalResultHandler;

impl ToolExecutor<ToolInvocation> for ReadTerminalResultHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("read_terminal_result")
    }

    fn spec(&self) -> ToolSpec {
        create_read_terminal_result_tool()
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(async move {
            let ToolInvocation {
                session, payload, ..
            } = invocation;
            let ToolPayload::Function { arguments } = payload else {
                return Err(FunctionCallError::RespondToModel(
                    "read_terminal_result handler received unsupported payload".to_string(),
                ));
            };
            let args: ReadTerminalResultArgs = parse_arguments(&arguments)?;
            let max_bytes = args
                .max_bytes
                .unwrap_or(DEFAULT_TERMINAL_RESULT_READ_MAX_BYTES)
                .clamp(1, MAX_TERMINAL_RESULT_READ_MAX_BYTES);
            let result = session
                .services
                .unified_exec_manager
                .read_terminal_result(&args.result_ref, args.offset, max_bytes)
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!("read_terminal_result failed: {err}"))
                })?;
            let content = serde_json::to_string(&result).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize read_terminal_result response: {err}"
                ))
            })?;
            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                content,
                Some(true),
            )))
        })
    }
}

impl CoreToolRuntime for ReadTerminalResultHandler {}
