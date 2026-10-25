use std::io;
use std::time::Duration;

/// TCP keepalive probes for a connected socket.
///
/// The operating system sends the first probe once the connection has been idle for `idle`, then
/// one every `interval`, and drops the connection after `retries` unanswered probes. This mirrors
/// libpq's `keepalives_idle`, `keepalives_interval` and `keepalives_count`. An unset interval or
/// retry count keeps the operating system default.
///
/// Without keepalive, a connection whose peer vanished without a FIN or RST, such as a database
/// host that stopped answering, looks open for as long as nothing is sent on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpKeepalive {
    idle: Duration,
    interval: Option<Duration>,
    retries: Option<u32>,
}

impl TcpKeepalive {
    /// Send the first probe after the connection has been idle for `idle`.
    pub const fn new(idle: Duration) -> Self {
        Self {
            idle,
            interval: None,
            retries: None,
        }
    }

    /// Send each further probe `interval` after the previous one.
    ///
    /// Ignored on platforms where the interval cannot be set.
    pub const fn interval(mut self, interval: Duration) -> Self {
        self.interval = Some(interval);
        self
    }

    /// Drop the connection after `retries` unanswered probes.
    ///
    /// Ignored on platforms where the retry count cannot be set.
    pub const fn retries(mut self, retries: u32) -> Self {
        self.retries = Some(retries);
        self
    }

    /// The idle time before the first probe.
    pub const fn get_idle(&self) -> Duration {
        self.idle
    }

    /// The time between probes, if set.
    pub const fn get_interval(&self) -> Option<Duration> {
        self.interval
    }

    /// The number of unanswered probes before the connection drops, if set.
    pub const fn get_retries(&self) -> Option<u32> {
        self.retries
    }

    // The platforms on which socket2 can set the probe interval and retry count.
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "emscripten",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "visionos",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "windows",
        target_os = "cygwin",
        target_os = "nuttx",
        all(target_os = "wasi", not(target_env = "p1")),
    ))]
    fn params(&self) -> socket2::TcpKeepalive {
        let mut params = socket2::TcpKeepalive::new().with_time(self.idle);
        if let Some(interval) = self.interval {
            params = params.with_interval(interval);
        }
        if let Some(retries) = self.retries {
            params = params.with_retries(retries);
        }
        params
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "emscripten",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "visionos",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "windows",
        target_os = "cygwin",
        target_os = "nuttx",
        all(target_os = "wasi", not(target_env = "p1")),
    )))]
    fn params(&self) -> socket2::TcpKeepalive {
        socket2::TcpKeepalive::new().with_time(self.idle)
    }
}

/// Socket options applied to a TCP connection as soon as it connects.
///
/// The default leaves every option at the operating system default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TcpOptions {
    keepalive: Option<TcpKeepalive>,
    user_timeout: Option<Duration>,
}

impl TcpOptions {
    /// Enable keepalive probes, or leave them off with `None`.
    pub const fn keepalive(mut self, keepalive: Option<TcpKeepalive>) -> Self {
        self.keepalive = keepalive;
        self
    }

    /// Set `TCP_USER_TIMEOUT`: drop the connection once sent data, keepalive probes included, has
    /// gone unacknowledged for `timeout`.
    ///
    /// This mirrors libpq's `tcp_user_timeout`. Without it, Linux retransmits unacknowledged data
    /// for about 15 minutes before it gives up on the connection. Only Linux, Android, Fuchsia and
    /// Cygwin support the option; elsewhere it is ignored.
    pub const fn user_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.user_timeout = timeout;
        self
    }

    /// The keepalive probes, if enabled.
    pub const fn get_keepalive(&self) -> Option<TcpKeepalive> {
        self.keepalive
    }

    /// The `TCP_USER_TIMEOUT`, if set.
    pub const fn get_user_timeout(&self) -> Option<Duration> {
        self.user_timeout
    }

    // Without a runtime feature, `connect_tcp_with` cannot connect, so nothing applies options.
    #[cfg_attr(
        not(any(feature = "_rt-tokio", feature = "_rt-async-io")),
        allow(dead_code)
    )]
    pub(super) fn apply(&self, socket: socket2::SockRef<'_>) -> io::Result<()> {
        if let Some(keepalive) = &self.keepalive {
            socket.set_tcp_keepalive(&keepalive.params())?;
        }
        #[cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "cygwin",
        ))]
        if let Some(timeout) = self.user_timeout {
            socket.set_tcp_user_timeout(Some(timeout))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    fn connected() -> (TcpStream, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (stream, listener)
    }

    #[test]
    fn default_options_leave_the_socket_alone() {
        let (stream, _listener) = connected();
        TcpOptions::default()
            .apply(socket2::SockRef::from(&stream))
            .unwrap();
        assert!(!socket2::SockRef::from(&stream).keepalive().unwrap());
    }

    #[test]
    fn keepalive_and_user_timeout_reach_the_socket() {
        let (stream, _listener) = connected();
        let keepalive = TcpKeepalive::new(Duration::from_secs(30))
            .interval(Duration::from_secs(10))
            .retries(3);
        TcpOptions::default()
            .keepalive(Some(keepalive))
            .user_timeout(Some(Duration::from_secs(60)))
            .apply(socket2::SockRef::from(&stream))
            .unwrap();

        let socket = socket2::SockRef::from(&stream);
        assert!(socket.keepalive().unwrap());
        assert_eq!(
            socket.tcp_keepalive_time().unwrap(),
            Duration::from_secs(30)
        );
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            assert_eq!(
                socket.tcp_keepalive_interval().unwrap(),
                Duration::from_secs(10)
            );
            assert_eq!(socket.tcp_keepalive_retries().unwrap(), 3);
        }
        #[cfg(target_os = "linux")]
        assert_eq!(
            socket.tcp_user_timeout().unwrap(),
            Some(Duration::from_secs(60))
        );
    }
}
