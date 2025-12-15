use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use bollard::{
    Docker,
    container::{
        Config as ContainerConfig, CreateContainerOptions, KillContainerOptions,
        StartContainerOptions,
    },
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    models::HostConfig,
};
use futures_util::StreamExt;
use thiserror::Error;
use tokio::{
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use tracing::{debug, warn};

use crate::config::CodeInterpreterConfig;

#[derive(Debug, Clone)]
pub struct CodeInterpreterSettings {
    pub docker_image: String,
    pub execution_timeout: Duration,
    pub memory_limit_mb: u64,
    pub cpu_percent: u64,
    pub max_sessions: usize,
    pub idle_timeout: Duration,
    pub preload_packages: Vec<String>,
}

impl From<CodeInterpreterConfig> for CodeInterpreterSettings {
    fn from(config: CodeInterpreterConfig) -> Self {
        Self {
            docker_image: config.docker_image,
            execution_timeout: Duration::from_secs(config.execution_timeout_secs.max(1)),
            memory_limit_mb: config.memory_limit_mb.max(64),
            cpu_percent: config.cpu_percent.clamp(1, 100),
            max_sessions: config.max_sessions.max(1),
            idle_timeout: if config.idle_timeout_secs == 0 {
                Duration::ZERO
            } else {
                Duration::from_secs(config.idle_timeout_secs)
            },
            preload_packages: config.preload_packages,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
}

#[derive(Debug)]
struct SandboxEntry {
    container_id: String,
    last_used: Instant,
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug, Error)]
pub enum CodeInterpreterError {
    #[error("Docker is unavailable: {0}")]
    DockerUnavailable(String),
    #[error("Failed to manage sandbox: {0}")]
    Sandbox(String),
    #[error("Failed to execute code: {0}")]
    Execution(String),
    #[error("Code execution timed out")]
    Timeout,
    #[error("Failed to acquire sandbox slot: {0}")]
    Pool(String),
}

pub struct CodeInterpreterService {
    docker: Docker,
    settings: CodeInterpreterSettings,
    sandboxes: Mutex<HashMap<String, SandboxEntry>>,
    limiter: Arc<Semaphore>,
}

impl CodeInterpreterService {
    pub async fn new(settings: CodeInterpreterSettings) -> Result<Self, CodeInterpreterError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|err| CodeInterpreterError::DockerUnavailable(err.to_string()))?;

        Ok(Self {
            docker,
            limiter: Arc::new(Semaphore::new(settings.max_sessions.max(1))),
            sandboxes: Mutex::new(HashMap::new()),
            settings,
        })
    }

    pub async fn execute(
        &self,
        session_id: &str,
        code: &str,
    ) -> Result<ExecutionResult, CodeInterpreterError> {
        self.reap_idle().await;

        let container_id = self.ensure_sandbox(session_id).await?;
        let result = self.exec_python(&container_id, code).await;
        if result.is_err() {
            self.teardown_session(session_id).await;
        }
        result
    }

    async fn ensure_sandbox(&self, session_id: &str) -> Result<String, CodeInterpreterError> {
        if let Some(existing) = self
            .sandboxes
            .lock()
            .await
            .get_mut(session_id)
            .map(|entry| {
                entry.last_used = Instant::now();
                entry.container_id.clone()
            })
        {
            return Ok(existing);
        }

        let permit = self
            .limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| CodeInterpreterError::Pool(err.to_string()))?;

        let container_id = match self.start_container(session_id).await {
            Ok(id) => id,
            Err(err) => {
                drop(permit);
                return Err(err);
            }
        };

        let mut guard = self.sandboxes.lock().await;
        guard.insert(
            session_id.to_string(),
            SandboxEntry {
                container_id: container_id.clone(),
                last_used: Instant::now(),
                _permit: permit,
            },
        );
        Ok(container_id)
    }

    async fn start_container(&self, session_id: &str) -> Result<String, CodeInterpreterError> {
        let sandbox_name = format!("nexus-ci-{session_id}");

        let host_config = HostConfig {
            memory: Some((self.settings.memory_limit_mb * 1024 * 1024) as i64),
            nano_cpus: Some(((self.settings.cpu_percent * 1_000_000_000) / 100) as i64),
            network_mode: Some("none".to_string()),
            auto_remove: Some(true),
            ..Default::default()
        };

        let container_config = ContainerConfig {
            image: Some(self.settings.docker_image.clone()),
            cmd: Some(vec!["sleep".into(), "infinity".into()]),
            working_dir: Some("/workspace".into()),
            host_config: Some(host_config),
            ..Default::default()
        };

        let container = self
            .docker
            .create_container(
                Some(CreateContainerOptions::<String> {
                    name: sandbox_name,
                    platform: None,
                }),
                container_config,
            )
            .await
            .map_err(|err| CodeInterpreterError::Sandbox(err.to_string()))?;

        self.docker
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|err| CodeInterpreterError::Sandbox(err.to_string()))?;

        if !self.settings.preload_packages.is_empty() {
            let mut install_cmd = vec![
                "pip".to_string(),
                "install".to_string(),
                "--no-cache-dir".to_string(),
            ];
            install_cmd.extend(self.settings.preload_packages.clone());
            let _ = self
                .run_exec(&container.id, install_cmd, Duration::from_secs(60))
                .await;
        }

        Ok(container.id)
    }

    async fn exec_python(
        &self,
        container_id: &str,
        code: &str,
    ) -> Result<ExecutionResult, CodeInterpreterError> {
        self.run_exec(
            container_id,
            vec!["python3".into(), "-u".into(), "-c".into(), code.to_string()],
            self.settings.execution_timeout,
        )
        .await
    }

    async fn run_exec(
        &self,
        container_id: &str,
        cmd: Vec<String>,
        timeout_window: Duration,
    ) -> Result<ExecutionResult, CodeInterpreterError> {
        let exec = self
            .docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    ..Default::default()
                },
            )
            .await
            .map_err(|err| CodeInterpreterError::Execution(err.to_string()))?;

        let stream = self
            .docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|err| CodeInterpreterError::Execution(err.to_string()))?;

        let collected = timeout(timeout_window, Self::collect_output(stream)).await;
        let (stdout, stderr) = match collected {
            Ok(result) => result?,
            Err(_) => return Err(CodeInterpreterError::Timeout),
        };

        let inspect = self
            .docker
            .inspect_exec(&exec.id)
            .await
            .map_err(|err| CodeInterpreterError::Execution(err.to_string()))?;

        let exit_code = inspect.exit_code.unwrap_or_default();

        Ok(ExecutionResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn collect_output(
        stream: StartExecResults,
    ) -> Result<(String, String), CodeInterpreterError> {
        let mut stdout = String::new();
        let mut stderr = String::new();

        if let StartExecResults::Attached { mut output, .. } = stream {
            while let Some(chunk) = output.next().await {
                match chunk.map_err(|err| CodeInterpreterError::Execution(err.to_string()))? {
                    bollard::container::LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }

        Ok((stdout, stderr))
    }

    async fn reap_idle(&self) {
        if self.settings.idle_timeout.is_zero() {
            return;
        }

        let idle_limit = self.settings.idle_timeout;
        let sessions: Vec<String> = self
            .sandboxes
            .lock()
            .await
            .iter()
            .filter(|(_, entry)| entry.last_used.elapsed() >= idle_limit)
            .map(|(session_id, _)| session_id.clone())
            .collect();

        for session_id in sessions {
            debug!("Shutting down idle interpreter session {session_id}");
            self.teardown_session(&session_id).await;
        }
    }

    async fn teardown_session(&self, session_id: &str) {
        let container_id = self
            .sandboxes
            .lock()
            .await
            .remove(session_id)
            .map(|entry| entry.container_id);

        if let Some(id) = container_id {
            self.kill_container(&id).await;
        }
    }

    async fn kill_container(&self, container_id: &str) {
        if let Err(err) = self
            .docker
            .kill_container(container_id, None::<KillContainerOptions<String>>)
            .await
        {
            warn!("Failed to stop interpreter container {container_id}: {err}");
        }
    }
}
