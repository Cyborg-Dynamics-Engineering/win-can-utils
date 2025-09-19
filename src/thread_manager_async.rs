use crosscan::can::CanFrame;
use std::io::ErrorKind;
use std::mem;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc::{Receiver, Sender};

/// Blocking helper: create PipeServer and wait for client connection
async fn create_server_and_wait(pipe_name: &str) -> std::io::Result<NamedPipeServer> {
    // Create server (this does NOT block)
    let server = ServerOptions::new().create(pipe_name)?;

    println!("Created server on: {:?}", pipe_name);

    // Wait until a client connects
    match server.connect().await {
        Ok(()) => Ok(server),
        Err(e) => Err(e),
    }
}

/// Start the IPC reader, creating and waiting for pipe server connection without blocking async runtime
pub async fn start_ipc_reader(channel_name: String, tx: Sender<Vec<u8>>) -> std::io::Result<()> {
    let pipe_name = format!(r"\\.\pipe\can_{}_in", channel_name);

    loop {
        let server = create_server_and_wait(&pipe_name).await?;

        let tx_clone = tx.clone();

        let mut reader = BufReader::new(server);
        loop {
            let mut buf = vec![0u8; mem::size_of::<CanFrame>()];

            // Try to read a full frame
            match reader.read_exact(&mut buf).await {
                Ok(_) => {
                    if tx_clone.send(buf).await.is_err() {
                        println!("Receiver closed");
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    println!("Pipe closed by client");
                    break;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

pub async fn start_ipc_writer(
    channel_name: String,
    mut rx: Receiver<Vec<u8>>,
) -> std::io::Result<()> {
    let pipe_name = format!(r"\\.\pipe\can_{}_out", channel_name.clone());
    let mut server = create_server_and_wait(&pipe_name).await?;

    loop {
        match rx.recv().await {
            Some(msg) => {
                if let Err(e) = server.write_all(&msg).await {
                    if e.kind() == ErrorKind::BrokenPipe {
                        println!("Client disconnected from IPC Writer");

                        // Restart the ipc_writer server and wait for another client to connect
                        server.shutdown().await?;
                        server = create_server_and_wait(&pipe_name).await?;
                    } else {
                        return Err(e);
                    }
                }

                match server.flush().await {
                    Ok(()) => (),
                    Err(e) => return Err(e),
                }
            }
            None => {
                // Channel closed: exit cleanly
                return Ok(());
            }
        }
    }
}
