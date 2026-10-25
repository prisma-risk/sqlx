use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Duration;

pub use buffered::{BufferedSocket, WriteBuffer};
use bytes::BufMut;
use cfg_if::cfg_if;

use crate::io::ReadBuf;

mod buffered;

pub trait Socket: Send + Sync + Unpin + 'static {
    fn try_read(&mut self, buf: &mut dyn ReadBuf) -> io::Result<usize>;

    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize>;

    fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    fn poll_flush(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // `flush()` is a no-op for TCP/UDS
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    fn read<'a, B: ReadBuf>(&'a mut self, buf: &'a mut B) -> Read<'a, Self, B>
    where
        Self: Sized,
    {
        Read { socket: self, buf }
    }

    fn write<'a>(&'a mut self, buf: &'a [u8]) -> Write<'a, Self>
    where
        Self: Sized,
    {
        Write { socket: self, buf }
    }

    fn flush(&mut self) -> Flush<'_, Self>
    where
        Self: Sized,
    {
        Flush { socket: self }
    }

    fn shutdown(&mut self) -> Shutdown<'_, Self>
    where
        Self: Sized,
    {
        Shutdown { socket: self }
    }
}

pub struct Read<'a, S: ?Sized, B> {
    socket: &'a mut S,
    buf: &'a mut B,
}

impl<S: ?Sized, B> Future for Read<'_, S, B>
where
    S: Socket,
    B: ReadBuf,
{
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        while this.buf.has_remaining_mut() {
            match this.socket.try_read(&mut *this.buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.socket.poll_read_ready(cx))?;
                }
                ready => return Poll::Ready(ready),
            }
        }

        Poll::Ready(Ok(0))
    }
}

pub struct Write<'a, S: ?Sized> {
    socket: &'a mut S,
    buf: &'a [u8],
}

impl<S: ?Sized> Future for Write<'_, S>
where
    S: Socket,
{
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;

        while !this.buf.is_empty() {
            match this.socket.try_write(this.buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ready!(this.socket.poll_write_ready(cx))?;
                }
                ready => return Poll::Ready(ready),
            }
        }

        Poll::Ready(Ok(0))
    }
}

pub struct Flush<'a, S: ?Sized> {
    socket: &'a mut S,
}

impl<S: Socket + ?Sized> Future for Flush<'_, S> {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.socket.poll_flush(cx)
    }
}

pub struct Shutdown<'a, S: ?Sized> {
    socket: &'a mut S,
}

impl<S: ?Sized> Future for Shutdown<'_, S>
where
    S: Socket,
{
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.socket.poll_shutdown(cx)
    }
}

pub trait WithSocket {
    type Output;

    fn with_socket<S: Socket>(self, socket: S) -> impl Future<Output = Self::Output> + Send;
}

pub struct SocketIntoBox;

impl WithSocket for SocketIntoBox {
    type Output = Box<dyn Socket>;

    async fn with_socket<S: Socket>(self, socket: S) -> Self::Output {
        Box::new(socket)
    }
}

impl<S: Socket + ?Sized> Socket for Box<S> {
    fn try_read(&mut self, buf: &mut dyn ReadBuf) -> io::Result<usize> {
        (**self).try_read(buf)
    }

    fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (**self).try_write(buf)
    }

    fn poll_read_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (**self).poll_read_ready(cx)
    }

    fn poll_write_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (**self).poll_write_ready(cx)
    }

    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (**self).poll_flush(cx)
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        (**self).poll_shutdown(cx)
    }
}

/// Options applied to a TCP socket as it is opened.
///
/// `TCP_NODELAY` is not represented here: it is always set.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct TcpConnectOptions {
    /// `TCP_USER_TIMEOUT`, or `None` to keep the operating system default.
    pub user_timeout: Option<Duration>,
}

impl TcpConnectOptions {
    /// Options that change nothing about the socket beyond the defaults SQLx always applies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `TCP_USER_TIMEOUT`: the kernel drops the connection once sent data has gone
    /// unacknowledged for `timeout`. `None` or a zero duration keeps the operating system
    /// default. The kernel counts whole milliseconds, so a shorter non-zero `timeout` becomes one.
    ///
    /// Only Linux, Android, Fuchsia and Cygwin have the option; elsewhere it is ignored.
    pub fn with_user_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.user_timeout = timeout.into();
        self
    }
}

pub async fn connect_tcp<Ws: WithSocket>(
    host: &str,
    port: u16,
    with_socket: Ws,
) -> crate::Result<Ws::Output> {
    connect_tcp_with(host, port, with_socket, &TcpConnectOptions::new()).await
}

/// Open a TCP socket to `host` and `port`, applying `options` to it.
pub async fn connect_tcp_with<Ws: WithSocket>(
    host: &str,
    port: u16,
    with_socket: Ws,
    options: &TcpConnectOptions,
) -> crate::Result<Ws::Output> {
    #[cfg(feature = "_rt-tokio")]
    if crate::rt::rt_tokio::available() {
        let socket = tokio::net::TcpStream::connect((host, port)).await?;
        socket.set_nodelay(true)?;
        set_tcp_user_timeout(&socket, options.user_timeout)?;
        return Ok(with_socket.with_socket(socket).await);
    }

    cfg_if! {
        if #[cfg(feature = "_rt-async-io")] {
            // Options are applied inside, on the concrete socket type.
            Ok(with_socket.with_socket(connect_tcp_async_io(host, port, options).await?).await)
        } else {
            crate::rt::missing_rt((host, port, with_socket, options))
        }
    }
}

/// Without `TCP_USER_TIMEOUT`, Linux retransmits unacknowledged data for about 15 minutes
/// (`tcp_retries2`) before it fails a connection whose server stopped answering without a FIN or
/// RST.
#[cfg(all(
    any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "cygwin"
    ),
    any(feature = "_rt-tokio", feature = "_rt-async-io")
))]
fn set_tcp_user_timeout<'a>(
    stream: impl Into<socket2::SockRef<'a>>,
    timeout: Option<Duration>,
) -> crate::Result<()> {
    let Some(timeout) = timeout else {
        return Ok(());
    };
    // socket2 truncates to whole milliseconds, and zero means the system default.
    let timeout = if timeout.is_zero() {
        timeout
    } else {
        std::cmp::max(timeout, Duration::from_millis(1))
    };

    stream
        .into()
        .set_tcp_user_timeout(Some(timeout))
        .map_err(|err| {
            crate::Error::Configuration(
                format!("failed to set TCP_USER_TIMEOUT to {timeout:?}: {err}").into(),
            )
        })
}

/// This target has no `TCP_USER_TIMEOUT`, so the option is ignored rather than failing the
/// connection.
#[cfg(all(
    not(any(
        target_os = "android",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "cygwin"
    )),
    any(feature = "_rt-tokio", feature = "_rt-async-io")
))]
fn set_tcp_user_timeout<S>(_stream: &S, _timeout: Option<Duration>) -> crate::Result<()> {
    Ok(())
}

/// Open a TCP socket to `host` and `port`.
///
/// If `host` is a hostname, attempt to connect to each address it resolves to.
///
/// This implements the same behavior as [`tokio::net::TcpStream::connect()`], additionally sets
/// the `TCP_NODELAY` flag, and applies `options`.
#[cfg(feature = "_rt-async-io")]
async fn connect_tcp_async_io(
    host: &str,
    port: u16,
    options: &TcpConnectOptions,
) -> crate::Result<impl Socket> {
    use async_io::Async;
    use std::net::{IpAddr, TcpStream, ToSocketAddrs};

    // IPv6 addresses in URLs will be wrapped in brackets and the `url` crate doesn't trim those.
    let host = host.trim_matches(&['[', ']'][..]);

    if let Ok(addr) = host.parse::<IpAddr>() {
        let socket = Async::<TcpStream>::connect((addr, port)).await?;
        socket.get_ref().set_nodelay(true)?;
        set_tcp_user_timeout(socket.get_ref(), options.user_timeout)?;
        return Ok(socket);
    }

    let host = host.to_string();

    let addresses = crate::rt::spawn_blocking(move || {
        let addr = (host.as_str(), port);
        ToSocketAddrs::to_socket_addrs(&addr)
    })
    .await?;

    let mut last_err = None;

    // Loop through all the Socket Addresses that the hostname resolves to
    for socket_addr in addresses {
        match Async::<TcpStream>::connect(socket_addr).await {
            Ok(stream) => {
                stream.get_ref().set_nodelay(true)?;
                set_tcp_user_timeout(stream.get_ref(), options.user_timeout)?;
                return Ok(stream);
            }
            Err(e) => last_err = Some(e),
        }
    }

    // If we reach this point, it means we failed to connect to any of the addresses.
    // Return the last error we encountered, or a custom error if the hostname didn't resolve to any address.
    Err(last_err
        .unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "Hostname did not resolve to any addresses",
            )
        })
        .into())
}

/// Connect a Unix Domain Socket at the given path.
///
/// Returns an error if Unix Domain Sockets are not supported on this platform.
pub async fn connect_uds<P: AsRef<Path>, Ws: WithSocket>(
    path: P,
    with_socket: Ws,
) -> crate::Result<Ws::Output> {
    #[cfg(unix)]
    {
        #[cfg(feature = "_rt-tokio")]
        if crate::rt::rt_tokio::available() {
            use tokio::net::UnixStream;

            let stream = UnixStream::connect(path).await?;

            return Ok(with_socket.with_socket(stream).await);
        }

        cfg_if! {
            if #[cfg(feature = "_rt-async-io")] {
                use async_io::Async;
                use std::os::unix::net::UnixStream;

                let stream = Async::<UnixStream>::connect(path).await?;

                Ok(with_socket.with_socket(stream).await)
            } else {
                crate::rt::missing_rt((path, with_socket))
            }
        }
    }

    #[cfg(not(unix))]
    {
        drop((path, with_socket));

        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unix domain sockets are not supported on this platform",
        )
        .into())
    }
}

#[cfg(all(
    test,
    target_os = "linux",
    any(feature = "_rt-tokio", feature = "_rt-async-io")
))]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn user_timeout_reaches_the_socket_and_none_leaves_the_default() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let socket = socket2::SockRef::from(&stream);
        let default = socket.tcp_user_timeout().unwrap();

        set_tcp_user_timeout(&stream, None).unwrap();
        assert_eq!(socket.tcp_user_timeout().unwrap(), default);

        set_tcp_user_timeout(&stream, Some(Duration::from_secs(60))).unwrap();
        assert_eq!(
            socket.tcp_user_timeout().unwrap(),
            Some(Duration::from_secs(60))
        );

        set_tcp_user_timeout(&stream, Some(Duration::from_micros(500))).unwrap();
        assert_eq!(
            socket.tcp_user_timeout().unwrap(),
            Some(Duration::from_millis(1)),
            "a sub-millisecond timeout must not truncate to the system default"
        );
    }

    #[cfg(feature = "_rt-tokio")]
    #[test]
    fn connect_tcp_with_applies_the_user_timeout() {
        struct UserTimeout;

        impl WithSocket for UserTimeout {
            type Output = Option<Duration>;

            async fn with_socket<S: Socket>(self, socket: S) -> Self::Output {
                let stream = (&socket as &dyn std::any::Any)
                    .downcast_ref::<tokio::net::TcpStream>()
                    .expect("the tokio path hands over a tokio TcpStream");
                socket2::SockRef::from(stream).tcp_user_timeout().unwrap()
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let options = TcpConnectOptions::new().with_user_timeout(Duration::from_secs(45));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let applied = runtime
            .block_on(connect_tcp_with("127.0.0.1", port, UserTimeout, &options))
            .unwrap();
        assert_eq!(applied, Some(Duration::from_secs(45)));
    }
}
