use std::net::{Ipv4Addr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub struct HttpService {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl HttpService {
    pub async fn new() -> std::io::Result<Self> {
        let mut service = Self {
            port: 0,
            shutdown: None,
            task: None,
        };
        service.start().await?;
        Ok(service)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_running(&self) -> bool {
        self.task
            .as_ref()
            .map(|task| !task.is_finished())
            .unwrap_or(false)
    }

    pub async fn start(&mut self) -> std::io::Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let bind_port = self.port;
        let listener =
            TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, bind_port))).await?;
        self.port = listener.local_addr()?.port();

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown = Some(shutdown_tx);
        self.task = Some(tokio::spawn(run_server(listener, shutdown_rx)));

        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for HttpService {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn run_server(listener: TcpListener, shutdown: oneshot::Receiver<()>) {
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                let Ok((mut socket, _)) = accepted else {
                    continue;
                };

                tokio::spawn(async move {
                    let mut buffer = [0_u8; 4096];
                    let _ = socket.read(&mut buffer).await;

                    let response = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/plain\r\n",
                        "Content-Length: 2\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "OK",
                    );

                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        }
    }
}
