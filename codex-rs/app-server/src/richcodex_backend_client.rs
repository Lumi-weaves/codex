use super::AppServerMessage;
use super::SHUTDOWN_TIMEOUT;
use super::read_shutdown_complete;
use super::write_message;
use std::io;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const COMMAND_CHANNEL_CAPACITY: usize = 16;

enum BackendCommand {
    Shutdown {
        response: oneshot::Sender<io::Result<()>>,
    },
}

#[derive(Clone)]
pub(super) struct RichCodexBackendClient {
    commands: mpsc::Sender<BackendCommand>,
}

impl RichCodexBackendClient {
    pub(super) fn spawn(
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
    ) -> (Self, JoinHandle<io::Result<()>>) {
        let (commands, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let client = Self { commands };
        let actor = tokio::spawn(run_backend_actor(child, stdin, stdout, command_rx));
        (client, actor)
    }

    pub(super) async fn shutdown(&self) -> io::Result<()> {
        let (response, received) = oneshot::channel();
        self.commands
            .send(BackendCommand::Shutdown { response })
            .await
            .map_err(|_| io::Error::other("RichCodex model backend is unavailable for shutdown"))?;
        received.await.unwrap_or_else(|_| {
            Err(io::Error::other(
                "RichCodex model backend shutdown channel closed",
            ))
        })
    }
}

async fn run_backend_actor(
    mut child: Child,
    mut stdin: ChildStdin,
    mut stdout: BufReader<ChildStdout>,
    mut commands: mpsc::Receiver<BackendCommand>,
) -> io::Result<()> {
    if let Some(BackendCommand::Shutdown { response }) = commands.recv().await {
        let result = shutdown_child(&mut child, &mut stdin, &mut stdout).await;
        let returned = result
            .as_ref()
            .map(|_| ())
            .map_err(|err| io::Error::new(err.kind(), err.to_string()));
        let _ = response.send(returned);
        return result;
    }

    stop_child(&mut child).await;
    Ok(())
}

async fn stop_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn shutdown_child(
    child: &mut Child,
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
) -> io::Result<()> {
    let request_id = "app-server-shutdown";
    write_message(stdin, &AppServerMessage::Shutdown { request_id }).await?;
    read_shutdown_complete(stdout, request_id, SHUTDOWN_TIMEOUT).await?;
    stdin.shutdown().await?;
    let status = match timeout(SHUTDOWN_TIMEOUT, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            stop_child(child).await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "model backend did not exit",
            ));
        }
    };
    if !status.success() {
        return Err(io::Error::other(format!(
            "model backend exited with {status}"
        )));
    }
    Ok(())
}
