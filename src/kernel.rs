use std::path::PathBuf;

use anyhow::{Context, Result};
use jupyter_protocol::{
    CodeMirrorMode, CommInfoReply, CompleteReply, DisplayData, ErrorOutput, ExecuteInput,
    ExecuteReply, ExecutionCount, HistoryReply, InspectReply, IoPubWelcome, IsCompleteReply,
    IsCompleteReplyStatus, JupyterMessage, JupyterMessageContent, KernelInfoReply, LanguageInfo,
    Media, MediaType, ReplyError, ReplyStatus, ShutdownReply, Status, StreamContent,
};
use runtimelib::{
    RouterRecvConnection, RouterSendConnection, SubscriptionEvent,
    create_kernel_control_connection, create_kernel_heartbeat_connection,
    create_kernel_iopub_xpub_connection, create_kernel_shell_connection,
    create_kernel_stdin_connection,
};
use uuid::Uuid;

use crate::DISPLAY_NAME;
use crate::typst_session::{ExecutionOutput, PageSetup, TypstSession};

pub async fn run(connection_file: PathBuf, page_setup: String) -> Result<()> {
    let bytes = std::fs::read(&connection_file)
        .with_context(|| format!("failed to read {}", connection_file.display()))?;
    let connection_info =
        serde_json::from_slice(&bytes).context("failed to parse connection file")?;
    let page_setup = PageSetup::parse(&page_setup)?;
    KernelServer::run(connection_info, page_setup).await
}

struct KernelServer {
    execution_count: ExecutionCount,
    iopub: runtimelib::KernelIoPubXPubConnection,
    shell: RouterSendConnection,
    typst: TypstSession,
}

impl KernelServer {
    async fn run(
        connection_info: jupyter_protocol::ConnectionInfo,
        page_setup: PageSetup,
    ) -> Result<()> {
        let session_id = Uuid::new_v4().to_string();
        let mut heartbeat = create_kernel_heartbeat_connection(&connection_info).await?;
        let shell_connection =
            create_kernel_shell_connection(&connection_info, &session_id).await?;
        let (shell_writer, shell_reader) = shell_connection.split();
        let mut control = create_kernel_control_connection(&connection_info, &session_id).await?;
        let _stdin = create_kernel_stdin_connection(&connection_info, &session_id).await?;
        let iopub = create_kernel_iopub_xpub_connection(&connection_info, &session_id).await?;

        let heartbeat_handle =
            tokio::spawn(async move { while heartbeat.single_heartbeat().await.is_ok() {} });

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let control_handle = tokio::spawn(async move {
            while let Ok(message) = control.read().await {
                match &message.content {
                    JupyterMessageContent::KernelInfoRequest(_) => {
                        let _ = control.send(kernel_info().as_child_of(&message)).await;
                    }
                    JupyterMessageContent::ShutdownRequest(req) => {
                        let reply = ShutdownReply {
                            restart: req.restart,
                            status: ReplyStatus::Ok,
                            error: None,
                        }
                        .as_child_of(&message);
                        let _ = control.send(reply).await;
                        let _ = shutdown_tx.send(());
                        return;
                    }
                    _ => {}
                }
            }
        });

        let mut kernel = Self {
            execution_count: ExecutionCount::new(0),
            iopub,
            shell: shell_writer,
            typst: TypstSession::new(page_setup),
        };
        let shell_handle =
            tokio::spawn(async move { kernel.shell_loop(shell_reader, shutdown_rx).await });

        tokio::select! {
            _ = heartbeat_handle => {}
            _ = control_handle => {}
            _ = shell_handle => {}
        }

        Ok(())
    }

    async fn shell_loop(
        &mut self,
        mut shell_reader: RouterRecvConnection,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                result = shell_reader.read() => {
                    match result {
                        Ok(message) => {
                            if let Err(error) = self.handle_shell_message(&message).await {
                                eprintln!("jupytypst kernel error: {error:#}");
                            }
                        }
                        Err(error) => {
                            eprintln!("jupytypst shell read error: {error}");
                            break;
                        }
                    }
                }
                result = self.iopub.recv_subscription() => {
                    match result {
                        Ok(SubscriptionEvent::Subscribe(topic)) => {
                            let welcome: JupyterMessage = IoPubWelcome::new(topic).into();
                            if let Err(error) = self.iopub.send(welcome).await {
                                eprintln!("jupytypst iopub welcome error: {error}");
                            }
                        }
                        Ok(SubscriptionEvent::Unsubscribe(_)) => {}
                        Err(error) => eprintln!("jupytypst iopub subscription error: {error}"),
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
    }

    async fn handle_shell_message(&mut self, parent: &JupyterMessage) -> Result<()> {
        self.iopub.send(Status::busy().as_child_of(parent)).await?;

        match &parent.content {
            JupyterMessageContent::KernelInfoRequest(_) => {
                self.shell.send(kernel_info().as_child_of(parent)).await?;
            }
            JupyterMessageContent::ExecuteRequest(request) => {
                self.handle_execute_request(&request.code, parent).await?;
            }
            JupyterMessageContent::CompleteRequest(request) => {
                let reply = CompleteReply {
                    matches: vec![],
                    cursor_start: request.cursor_pos,
                    cursor_end: request.cursor_pos,
                    metadata: Default::default(),
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::InspectRequest(_) => {
                let reply = InspectReply {
                    found: false,
                    data: Media::default(),
                    metadata: Default::default(),
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::IsCompleteRequest(_) => {
                let reply = IsCompleteReply {
                    status: IsCompleteReplyStatus::Complete,
                    indent: String::new(),
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::HistoryRequest(_) => {
                let reply = HistoryReply {
                    history: vec![],
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::CommInfoRequest(_) => {
                let reply = CommInfoReply {
                    status: ReplyStatus::Ok,
                    comms: Default::default(),
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::ShutdownRequest(request) => {
                let reply = ShutdownReply {
                    restart: request.restart,
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            _ => {}
        }

        self.iopub.send(Status::idle().as_child_of(parent)).await?;
        Ok(())
    }

    async fn handle_execute_request(&mut self, code: &str, parent: &JupyterMessage) -> Result<()> {
        self.execution_count.0 += 1;
        let execution_count = self.execution_count;

        self.iopub
            .send(
                ExecuteInput {
                    code: code.to_string(),
                    execution_count,
                }
                .as_child_of(parent),
            )
            .await?;

        let reply = match self.typst.execute(code) {
            Ok(result) => {
                for warning in result.warnings {
                    self.iopub
                        .send(StreamContent::stderr(&format!("{warning}\n")).as_child_of(parent))
                        .await?;
                }
                self.publish_output(result.output, parent).await?;
                ExecuteReply {
                    status: ReplyStatus::Ok,
                    execution_count,
                    payload: vec![],
                    user_expressions: None,
                    error: None,
                }
            }
            Err(error) => {
                let evalue = error.to_string();
                let error_output = ErrorOutput {
                    ename: "TypstError".to_string(),
                    evalue: evalue.clone(),
                    traceback: vec![evalue.clone()],
                };
                self.iopub.send(error_output.as_child_of(parent)).await?;
                ExecuteReply {
                    status: ReplyStatus::Error,
                    execution_count,
                    payload: vec![],
                    user_expressions: None,
                    error: Some(Box::new(ReplyError {
                        ename: "TypstError".to_string(),
                        evalue,
                        traceback: vec![],
                    })),
                }
            }
        };
        self.shell.send(reply.as_child_of(parent)).await?;
        Ok(())
    }

    async fn publish_output(
        &mut self,
        output: ExecutionOutput,
        parent: &JupyterMessage,
    ) -> Result<()> {
        let media = match output {
            ExecutionOutput::Svg(html) => Media::new(vec![
                MediaType::Html(html),
                MediaType::Plain("<svg>".to_string()),
            ]),
            ExecutionOutput::Html(html) => Media::new(vec![
                MediaType::Html(html),
                MediaType::Plain("<html>".to_string()),
            ]),
        };
        self.iopub
            .send(DisplayData::new(media).as_child_of(parent))
            .await?;
        Ok(())
    }
}

fn kernel_info() -> KernelInfoReply {
    KernelInfoReply {
        status: ReplyStatus::Ok,
        protocol_version: "5.3".to_string(),
        implementation: "jupytypst".to_string(),
        implementation_version: env!("CARGO_PKG_VERSION").to_string(),
        language_info: LanguageInfo {
            name: "typst-code".to_string(),
            version: "0.14".to_string(),
            mimetype: Some("text/x-typst-code".to_string()),
            file_extension: Some(".typc".to_string()),
            pygments_lexer: Some("typst-code".to_string()),
            codemirror_mode: Some(CodeMirrorMode::Simple("typst-code".to_string())),
            nbconvert_exporter: None,
        },
        banner: DISPLAY_NAME.to_string(),
        help_links: vec![],
        debugger: false,
        error: None,
    }
}
