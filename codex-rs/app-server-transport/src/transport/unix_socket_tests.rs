use super::AppServerTransport;
use super::CHANNEL_CAPACITY;
use super::TransportEvent;
use super::acquire_app_server_startup_lock;
use super::app_server_control_socket_path;
use super::start_control_socket_acceptor;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::JSONRPCNotification;
use codex_core::config::find_codex_home;
use codex_uds::UnixStream;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::SinkExt;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use std::io::Result as IoResult;
use std::path::Path;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::Bytes;
use tokio_tungstenite::tungstenite::Message as WebSocketMessage;
use tokio_util::sync::CancellationToken;

#[test]
fn listen_unix_socket_parses_as_unix_socket_transport() {
    assert_eq!(
        AppServerTransport::from_listen_url("unix://"),
        Ok(AppServerTransport::UnixSocket {
            socket_path: default_control_socket_path()
        })
    );
}

#[test]
fn listen_unix_socket_accepts_absolute_custom_path() {
    assert_eq!(
        AppServerTransport::from_listen_url("unix:///tmp/codex.sock"),
        Ok(AppServerTransport::UnixSocket {
            socket_path: absolute_path("/tmp/codex.sock")
        })
    );
}

#[test]
fn listen_unix_socket_accepts_relative_custom_path() {
    assert_eq!(
        AppServerTransport::from_listen_url("unix://codex.sock"),
        Ok(AppServerTransport::UnixSocket {
            socket_path: AbsolutePathBuf::relative_to_current_dir("codex.sock")
                .expect("relative path should resolve")
        })
    );
}

#[test]
fn control_socket_path_uses_codex_home_when_path_fits() {
    assert_eq!(
        app_server_control_socket_path(Path::new("/tmp/codex-home"))
            .expect("control socket path should resolve"),
        absolute_path("/tmp/codex-home/app-server-control/app-server-control.sock")
    );
}

#[cfg(unix)]
#[test]
fn control_socket_path_uses_short_tmp_path_when_codex_home_is_too_long() {
    use std::os::unix::ffi::OsStrExt;

    let codex_home = Path::new("/private/var/folders")
        .join("very-long-test-suite-temp-root")
        .join("interpreter-auth-reference-home-1780329159071214000")
        .join("open-interpreter-app-server-control-test-home")
        .join("nested-open-interpreter-home");
    let socket_path =
        app_server_control_socket_path(&codex_home).expect("control socket path should resolve");

    assert!(
        socket_path
            .as_path()
            .parent()
            .and_then(Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with("codex-app-server-control-"))
    );
    assert_eq!(socket_path.as_path().file_name().unwrap(), "control.sock");
    assert!(socket_path.as_path().as_os_str().as_bytes().len() < 100);
    assert_eq!(
        socket_path,
        app_server_control_socket_path(&codex_home).expect("control socket path should resolve")
    );
    assert_ne!(
        socket_path,
        app_server_control_socket_path(&codex_home.join("other"))
            .expect("control socket path should resolve")
    );
}

#[tokio::test]
async fn control_socket_acceptor_upgrades_and_forwards_websocket_text_messages_and_pings() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let socket_path = test_socket_path(temp_dir.path());
    let (transport_event_tx, mut transport_event_rx) =
        mpsc::channel::<TransportEvent>(CHANNEL_CAPACITY);
    let shutdown_token = CancellationToken::new();
    let accept_handle = start_control_socket_acceptor(
        socket_path.clone(),
        transport_event_tx,
        shutdown_token.clone(),
    )
    .await
    .expect("control socket acceptor should start");

    let stream = connect_to_socket(socket_path.as_path())
        .await
        .expect("client should connect");
    let (mut websocket, response) = client_async("ws://localhost/rpc", stream)
        .await
        .expect("websocket upgrade should complete");
    assert_eq!(response.status().as_u16(), 101);

    let opened = timeout(Duration::from_secs(1), transport_event_rx.recv())
        .await
        .expect("connection opened event should arrive")
        .expect("connection opened event");
    let connection_id = match opened {
        TransportEvent::ConnectionOpened { connection_id, .. } => connection_id,
        _ => panic!("expected connection opened event"),
    };

    let notification = JSONRPCMessage::Notification(JSONRPCNotification {
        method: "initialized".to_string(),
        params: None,
    });
    websocket
        .send(WebSocketMessage::Text(
            serde_json::to_string(&notification)
                .expect("notification should serialize")
                .into(),
        ))
        .await
        .expect("notification should send");

    let incoming = timeout(Duration::from_secs(1), transport_event_rx.recv())
        .await
        .expect("incoming message event should arrive")
        .expect("incoming message event");
    assert_eq!(
        match incoming {
            TransportEvent::IncomingMessage {
                connection_id: incoming_connection_id,
                message,
            } => (incoming_connection_id, message),
            _ => panic!("expected incoming message event"),
        },
        (connection_id, notification)
    );

    websocket
        .send(WebSocketMessage::Ping(Bytes::from_static(b"check")))
        .await
        .expect("ping should send");
    let pong = timeout(Duration::from_secs(1), websocket.next())
        .await
        .expect("pong should arrive")
        .expect("pong frame")
        .expect("pong should be valid");
    assert_eq!(pong, WebSocketMessage::Pong(Bytes::from_static(b"check")));

    websocket.close(None).await.expect("close should send");
    let closed = timeout(Duration::from_secs(1), transport_event_rx.recv())
        .await
        .expect("connection closed event should arrive")
        .expect("connection closed event");
    assert!(matches!(
        closed,
        TransportEvent::ConnectionClosed {
            connection_id: closed_connection_id,
        } if closed_connection_id == connection_id
    ));

    shutdown_token.cancel();
    accept_handle.await.expect("acceptor should join");
    assert_socket_path_removed(socket_path.as_path());
}

#[tokio::test]
async fn app_server_startup_lock_serializes_waiters() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let lock_path = test_startup_lock_path(temp_dir.path());
    let first_lock = acquire_app_server_startup_lock(lock_path.clone())
        .await
        .expect("first startup lock should succeed");
    let mut second_lock = tokio::spawn(acquire_app_server_startup_lock(lock_path));

    assert!(
        timeout(Duration::from_millis(100), &mut second_lock)
            .await
            .is_err()
    );

    drop(first_lock);
    second_lock
        .await
        .expect("second startup lock task should join")
        .expect("second startup lock should succeed");
}

#[cfg(unix)]
#[tokio::test]
async fn control_socket_file_is_private_after_bind() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let socket_path = test_socket_path(temp_dir.path());
    let (transport_event_tx, _transport_event_rx) =
        mpsc::channel::<TransportEvent>(CHANNEL_CAPACITY);
    let shutdown_token = CancellationToken::new();
    let accept_handle = start_control_socket_acceptor(
        socket_path.clone(),
        transport_event_tx,
        shutdown_token.clone(),
    )
    .await
    .expect("control socket acceptor should start");

    let metadata = tokio::fs::metadata(socket_path.as_path())
        .await
        .expect("socket metadata should exist");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    shutdown_token.cancel();
    accept_handle.await.expect("acceptor should join");
}

fn absolute_path(path: &str) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(path).expect("absolute path")
}

fn default_control_socket_path() -> AbsolutePathBuf {
    let codex_home = find_codex_home().expect("codex home");
    app_server_control_socket_path(&codex_home).expect("default control socket path")
}

fn test_socket_path(temp_dir: &Path) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(
        temp_dir
            .join("app-server-control")
            .join("app-server-control.sock"),
    )
    .expect("socket path should resolve")
}

fn test_startup_lock_path(temp_dir: &Path) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(
        temp_dir
            .join("app-server-control")
            .join("app-server-startup.lock"),
    )
    .expect("startup lock path should resolve")
}

async fn connect_to_socket(socket_path: &Path) -> IoResult<UnixStream> {
    UnixStream::connect(socket_path).await
}

#[cfg(unix)]
fn assert_socket_path_removed(socket_path: &Path) {
    assert!(!socket_path.exists());
}

#[cfg(windows)]
fn assert_socket_path_removed(_socket_path: &Path) {
    // uds_windows uses a regular filesystem path as its rendezvous point,
    // but there is no Unix socket filesystem node to assert on.
}
