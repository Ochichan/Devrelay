//! Local IPC transport primitives.
//!
//! M2 IPC starts with a small transport layer: local-user sockets, bounded
//! length-prefixed messages, and explicit timeout behavior. JSON-RPC is layered
//! above this module in later milestones.

use crate::{DevRelayError, Result};
use std::path::Path;
use std::time::Duration;

const LENGTH_PREFIX_BYTES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcLimits {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_message_bytes: usize,
}

impl Default for IpcLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(30),
            max_message_bytes: 1024 * 1024,
        }
    }
}

impl IpcLimits {
    fn validate(self) -> Result<()> {
        if self.connect_timeout == Duration::ZERO {
            return Err(ipc_error("IPC connect timeout must be greater than zero"));
        }
        if self.request_timeout == Duration::ZERO {
            return Err(ipc_error("IPC request timeout must be greater than zero"));
        }
        if self.max_message_bytes == 0 {
            return Err(ipc_error("IPC max message size must be greater than zero"));
        }
        Ok(())
    }
}

pub trait IpcConnection {
    fn peer_credentials(&self) -> Result<Option<PeerCredentials>>;
    fn read_message(&mut self, limits: IpcLimits) -> Result<Vec<u8>>;
    fn write_message(&mut self, payload: &[u8], limits: IpcLimits) -> Result<()>;
}

pub trait IpcTransport {
    type Connection: IpcConnection;

    fn endpoint(&self) -> &Path;
    fn accept(&self) -> Result<Self::Connection>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    pub uid: u32,
    pub gid: u32,
    pub pid: Option<u32>,
}

impl PeerCredentials {
    pub fn is_current_user(self) -> bool {
        self.uid == current_uid()
    }
}

fn ipc_error(detail: impl Into<String>) -> DevRelayError {
    DevRelayError::Ipc(detail.into())
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and does not write through pointers.
        unsafe { libc::geteuid() as u32 }
    }

    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::fs;
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;

    #[derive(Debug)]
    pub struct UnixIpcListener {
        listener: UnixListener,
        path: PathBuf,
    }

    impl UnixIpcListener {
        pub fn bind(path: impl AsRef<Path>) -> Result<Self> {
            let path = path.as_ref();
            prepare_socket_path(path)?;
            let listener = UnixListener::bind(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(Self {
                listener,
                path: path.to_path_buf(),
            })
        }
    }

    impl IpcTransport for UnixIpcListener {
        type Connection = UnixIpcConnection;

        fn endpoint(&self) -> &Path {
            &self.path
        }

        fn accept(&self) -> Result<Self::Connection> {
            let (stream, _) = self.listener.accept()?;
            let connection = UnixIpcConnection::from_stream(stream)?;
            connection.ensure_peer_is_current_user()?;
            Ok(connection)
        }
    }

    impl Drop for UnixIpcListener {
        fn drop(&mut self) {
            if is_socket_path(&self.path).unwrap_or(false) {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    #[derive(Debug)]
    pub struct UnixIpcConnection {
        stream: UnixStream,
    }

    impl UnixIpcConnection {
        pub fn connect(path: impl AsRef<Path>, limits: IpcLimits) -> Result<Self> {
            limits.validate()?;
            let path = path.as_ref().to_path_buf();
            let (tx, rx) = mpsc::sync_channel(1);
            let _join_handle = thread::Builder::new()
                .name("devrelay-ipc-connect".to_string())
                .spawn(move || {
                    let _ = tx.send(UnixStream::connect(path));
                })
                .map_err(|err| ipc_error(format!("failed to spawn IPC connect worker: {err}")))?;

            match rx.recv_timeout(limits.connect_timeout) {
                Ok(Ok(stream)) => Self::from_stream(stream),
                Ok(Err(err)) => Err(err.into()),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    Err(ipc_error("timed out connecting to local IPC socket"))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(ipc_error(
                    "IPC connect worker exited before returning a socket",
                )),
            }
        }

        pub fn from_stream(stream: UnixStream) -> Result<Self> {
            Ok(Self { stream })
        }

        fn ensure_peer_is_current_user(&self) -> Result<()> {
            if let Some(credentials) = self.peer_credentials()?
                && !credentials.is_current_user()
            {
                return Err(ipc_error(format!(
                    "rejected IPC peer uid {} for current uid {}",
                    credentials.uid,
                    current_uid()
                )));
            }
            Ok(())
        }
    }

    impl IpcConnection for UnixIpcConnection {
        fn peer_credentials(&self) -> Result<Option<PeerCredentials>> {
            peer_credentials(self.stream.as_raw_fd())
        }

        fn read_message(&mut self, limits: IpcLimits) -> Result<Vec<u8>> {
            limits.validate()?;
            self.stream.set_read_timeout(Some(limits.request_timeout))?;

            let mut len_bytes = [0_u8; LENGTH_PREFIX_BYTES];
            read_exact_or_ipc(
                &mut self.stream,
                &mut len_bytes,
                "malformed IPC message: missing length prefix",
            )?;
            let len = u32::from_be_bytes(len_bytes) as usize;
            if len > limits.max_message_bytes {
                return Err(ipc_error(format!(
                    "IPC message size {len} exceeds configured maximum {}",
                    limits.max_message_bytes
                )));
            }

            let mut payload = vec![0_u8; len];
            read_exact_or_ipc(
                &mut self.stream,
                &mut payload,
                "malformed IPC message: truncated payload",
            )?;
            Ok(payload)
        }

        fn write_message(&mut self, payload: &[u8], limits: IpcLimits) -> Result<()> {
            limits.validate()?;
            if payload.len() > limits.max_message_bytes {
                return Err(ipc_error(format!(
                    "IPC message size {} exceeds configured maximum {}",
                    payload.len(),
                    limits.max_message_bytes
                )));
            }
            let len = u32::try_from(payload.len())
                .map_err(|_| ipc_error("IPC message is too large to encode"))?;
            self.stream
                .set_write_timeout(Some(limits.request_timeout))?;
            write_all_or_ipc(&mut self.stream, &len.to_be_bytes())?;
            write_all_or_ipc(&mut self.stream, payload)?;
            Ok(())
        }
    }

    fn prepare_socket_path(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            return Ok(());
        }
        if !is_socket_path(path)? {
            return Err(ipc_error(format!(
                "IPC path exists and is not a Unix socket: {}",
                path.display()
            )));
        }

        match UnixStream::connect(path) {
            Ok(_) => Err(ipc_error(format!(
                "active IPC socket already exists: {}",
                path.display()
            ))),
            Err(err) if err.kind() == ErrorKind::PermissionDenied => Err(ipc_error(format!(
                "cannot inspect existing IPC socket {}: {err}",
                path.display()
            ))),
            Err(_) => {
                fs::remove_file(path)?;
                Ok(())
            }
        }
    }

    fn is_socket_path(path: &Path) -> Result<bool> {
        Ok(fs::symlink_metadata(path)?.file_type().is_socket())
    }

    fn read_exact_or_ipc(
        stream: &mut UnixStream,
        buffer: &mut [u8],
        malformed_message: &'static str,
    ) -> Result<()> {
        match stream.read_exact(buffer) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => Err(ipc_error(malformed_message)),
            Err(err) if is_timeout(&err) => Err(ipc_error("timed out reading IPC message")),
            Err(err) => Err(err.into()),
        }
    }

    fn write_all_or_ipc(stream: &mut UnixStream, buffer: &[u8]) -> Result<()> {
        match stream.write_all(buffer) {
            Ok(()) => Ok(()),
            Err(err) if is_timeout(&err) => Err(ipc_error("timed out writing IPC message")),
            Err(err) => Err(err.into()),
        }
    }

    fn is_timeout(err: &std::io::Error) -> bool {
        matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
    }

    #[cfg(target_os = "linux")]
    fn peer_credentials(fd: RawFd) -> Result<Option<PeerCredentials>> {
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: fd is owned by a live UnixStream, credentials points to enough
        // writable memory for libc::ucred, and len is initialized to that size.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast::<libc::c_void>(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: getsockopt returned success and initialized the ucred value.
        let credentials = unsafe { credentials.assume_init() };
        Ok(Some(PeerCredentials {
            uid: credentials.uid,
            gid: credentials.gid,
            pid: Some(credentials.pid as u32),
        }))
    }

    #[cfg(target_os = "macos")]
    fn peer_credentials(fd: RawFd) -> Result<Option<PeerCredentials>> {
        let mut uid = 0 as libc::uid_t;
        let mut gid = 0 as libc::gid_t;
        // SAFETY: fd is owned by a live UnixStream and uid/gid are valid output
        // pointers for getpeereid.
        let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Some(PeerCredentials {
            uid,
            gid,
            pid: None,
        }))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn peer_credentials(_fd: RawFd) -> Result<Option<PeerCredentials>> {
        Ok(None)
    }
}

#[cfg(unix)]
pub use unix::{UnixIpcConnection, UnixIpcListener};

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream};
    use std::thread;
    use std::time::Duration;

    fn socket_path(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(name);
        (temp, path)
    }

    #[test]
    fn unix_transport_round_trips_messages_and_checks_peer_credentials() {
        let (_temp, path) = socket_path("agent.sock");
        let server = UnixIpcListener::bind(&path).unwrap();
        let endpoint = server.endpoint().to_path_buf();
        let handle = thread::spawn(move || {
            let mut connection = server.accept().unwrap();
            let credentials = connection.peer_credentials().unwrap().unwrap();
            assert!(credentials.is_current_user());
            let request = connection.read_message(IpcLimits::default()).unwrap();
            assert_eq!(request, b"ping");
            connection
                .write_message(b"pong", IpcLimits::default())
                .unwrap();
        });

        let mut client = UnixIpcConnection::connect(&endpoint, IpcLimits::default()).unwrap();
        client.write_message(b"ping", IpcLimits::default()).unwrap();
        assert_eq!(client.read_message(IpcLimits::default()).unwrap(), b"pong");
        handle.join().unwrap();
    }

    #[test]
    fn bind_removes_stale_socket_file() {
        let (_temp, path) = socket_path("stale.sock");
        let raw = StdUnixListener::bind(&path).unwrap();
        drop(raw);
        for _ in 0..100 {
            if UnixStream::connect(&path).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(path.exists());

        let listener = UnixIpcListener::bind(&path).unwrap();

        assert_eq!(listener.endpoint(), path.as_path());
    }

    #[test]
    fn bind_refuses_active_socket() {
        let (_temp, path) = socket_path("active.sock");
        let _raw = StdUnixListener::bind(&path).unwrap();

        let err = UnixIpcListener::bind(&path).unwrap_err();

        assert!(matches!(err, DevRelayError::Ipc(_)));
        assert!(err.to_string().contains("active IPC socket"));
    }

    #[test]
    fn bind_refuses_non_socket_path() {
        let (_temp, path) = socket_path("regular-file.sock");
        std::fs::write(&path, "not a socket").unwrap();

        let err = UnixIpcListener::bind(&path).unwrap_err();

        assert!(matches!(err, DevRelayError::Ipc(_)));
        assert!(err.to_string().contains("not a Unix socket"));
    }

    #[test]
    fn write_rejects_oversized_message() {
        let (left, _right) = UnixStream::pair().unwrap();
        let mut connection = UnixIpcConnection::from_stream(left).unwrap();
        let limits = IpcLimits {
            max_message_bytes: 3,
            ..IpcLimits::default()
        };

        let err = connection.write_message(b"four", limits).unwrap_err();

        assert!(matches!(err, DevRelayError::Ipc(_)));
        assert!(err.to_string().contains("exceeds configured maximum"));
    }

    #[test]
    fn read_rejects_oversized_message_length() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(&4_u32.to_be_bytes()).unwrap();
        let mut connection = UnixIpcConnection::from_stream(reader).unwrap();
        let limits = IpcLimits {
            max_message_bytes: 3,
            ..IpcLimits::default()
        };

        let err = connection.read_message(limits).unwrap_err();

        assert!(matches!(err, DevRelayError::Ipc(_)));
        assert!(err.to_string().contains("exceeds configured maximum"));
    }

    #[test]
    fn read_rejects_truncated_payload_as_malformed() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(&4_u32.to_be_bytes()).unwrap();
        writer.write_all(b"xy").unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        let mut connection = UnixIpcConnection::from_stream(reader).unwrap();

        let err = connection.read_message(IpcLimits::default()).unwrap_err();

        assert!(matches!(err, DevRelayError::Ipc(_)));
        assert!(err.to_string().contains("truncated payload"));
    }
}
