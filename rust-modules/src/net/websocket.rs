//! Persistent raw connections over the process libcurl easy handle.
//!
//! Connect-only performs TCP and optional TLS, then leaves the byte protocol to the caller.
//! The handle owns the socket and must outlive every send and receive.

use super::{
    available, curl_easy_getinfo_socket, curl_easy_init, curl_easy_perform,
    curl_easy_recv, curl_easy_send, curl_easy_setopt_long, curl_easy_setopt_ptr, curl_slist_append,
    resolve, threaded_tls_ready, Easy, HeaderList, CURLINFO_ACTIVESOCKET, CURLPROTO_HTTP,
    CURLPROTO_HTTPS, CURLOPT_CONNECTTIMEOUT_MS, CURLOPT_CONNECT_ONLY, CURLOPT_NOSIGNAL,
    CURLOPT_PROTOCOLS, CURLOPT_RESOLVE, CURLOPT_SSL_VERIFYHOST, CURLOPT_SSL_VERIFYPEER,
    CURLOPT_TIMEOUT_MS, CURLOPT_URL,
};
use crate::origin::Origin;
use std::ffi::CString;
use std::io;
use std::os::raw::{c_int, c_long, c_short, c_void};
use std::ptr;
use std::time::{Duration, Instant};

const CURLE_AGAIN: c_int = 81;

pub(crate) struct RawConnection {
    easy: Easy,
    fd: c_int,
}

impl RawConnection {
    pub(crate) fn connect(origin: &Origin) -> io::Result<Self> {
        if !available() {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "libcurl unavailable"));
        }
        if origin.is_tls() && !threaded_tls_ready() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "concurrent TLS unavailable",
            ));
        }
        if super::refuse_name(origin.host(), 3) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "WAN refused"));
        }
        let url = CString::new(origin.base())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid origin"))?;
        let resolve_entry = resolve::system_entry(origin.host(), origin.port())
            .map(CString::new)
            .transpose()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid resolve entry"))?;
        unsafe {
            let handle = curl_easy_init();
            if handle.is_null() {
                return Err(io::Error::new(io::ErrorKind::OutOfMemory, "curl_easy_init"));
            }
            let easy = Easy(handle);
            macro_rules! require {
                ($call:expr) => {{
                    let rc = $call;
                    if rc != 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            format!("libcurl option {} refused: {rc}", stringify!($call)),
                        ));
                    }
                }};
            }
            require!(curl_easy_setopt_ptr(
                easy.0,
                CURLOPT_URL,
                url.as_ptr() as *const c_void
            ));
            require!(curl_easy_setopt_long(
                easy.0,
                CURLOPT_PROTOCOLS,
                CURLPROTO_HTTP | CURLPROTO_HTTPS
            ));
            require!(curl_easy_setopt_long(easy.0, CURLOPT_SSL_VERIFYPEER, 1 as c_long));
            require!(curl_easy_setopt_long(easy.0, CURLOPT_SSL_VERIFYHOST, 2 as c_long));
            require!(curl_easy_setopt_long(easy.0, CURLOPT_NOSIGNAL, 1 as c_long));
            require!(curl_easy_setopt_long(easy.0, CURLOPT_CONNECTTIMEOUT_MS, 3_000));
            require!(curl_easy_setopt_long(easy.0, CURLOPT_TIMEOUT_MS, 4_000));
            require!(curl_easy_setopt_long(easy.0, CURLOPT_CONNECT_ONLY, 1 as c_long));
            let mut resolve_list = HeaderList(ptr::null_mut());
            if let Some(entry) = resolve_entry.as_ref() {
                let list = curl_slist_append(ptr::null_mut(), entry.as_ptr());
                if list.is_null() {
                    return Err(io::Error::new(io::ErrorKind::OutOfMemory, "resolve list"));
                }
                resolve_list.0 = list;
                require!(curl_easy_setopt_ptr(
                    easy.0,
                    CURLOPT_RESOLVE,
                    list as *const c_void
                ));
            }
            let rc = curl_easy_perform(easy.0);
            if rc != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("libcurl connect failed: {rc}"),
                ));
            }
            let mut socket: c_int = -1;
            require!(curl_easy_getinfo_socket(
                easy.0,
                CURLINFO_ACTIVESOCKET,
                &mut socket as *mut c_int
            ));
            let fd = if socket >= 0 {
                socket
            } else {
                return Err(io::Error::new(io::ErrorKind::NotConnected, "no active socket"));
            };
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { easy, fd })
        }
    }

    pub(crate) fn fd(&self) -> c_int {
        self.fd
    }

    pub(crate) fn send_all(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        let mut sent = 0;
        while sent < bytes.len() {
            match self.try_send(&bytes[sent..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "zero send")),
                Ok(n) => sent += n,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait(libc::POLLOUT, remaining(deadline)?)?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn read_some(&mut self, out: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.try_recv(out) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.wait(libc::POLLIN, remaining(deadline)?)?;
                }
                result => return result,
            }
        }
    }

    fn try_send(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut sent = 0_usize;
        let rc = unsafe {
            curl_easy_send(
                self.easy.0,
                bytes.as_ptr() as *const c_void,
                bytes.len(),
                &mut sent as *mut usize,
            )
        };
        curl_io_result(rc, sent)
    }

    fn try_recv(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut read = 0_usize;
        let rc = unsafe {
            curl_easy_recv(
                self.easy.0,
                out.as_mut_ptr() as *mut c_void,
                out.len(),
                &mut read as *mut usize,
            )
        };
        curl_io_result(rc, read)
    }

    fn wait(&self, events: c_short, timeout: Duration) -> io::Result<()> {
        let mut pollfd = libc::pollfd {
            fd: self.fd,
            events,
            revents: 0,
        };
        let millis = timeout.as_millis().min(i32::MAX as u128) as c_int;
        let rc = unsafe { libc::poll(&mut pollfd as *mut libc::pollfd, 1, millis) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        if rc == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "socket wait"));
        }
        if has_ready_event(pollfd.revents, events) {
            return Ok(());
        }
        if pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "socket closed"));
        }
        Ok(())
    }
}

impl Drop for RawConnection {
    fn drop(&mut self) {
        unsafe {
            libc::shutdown(self.fd, libc::SHUT_RDWR);
        }
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "socket deadline"))
}

pub(crate) fn has_ready_event(revents: c_short, events: c_short) -> bool {
    revents & events != 0
}

fn curl_io_result(rc: c_int, count: usize) -> io::Result<usize> {
    match rc {
        0 => Ok(count),
        CURLE_AGAIN => Err(io::Error::new(io::ErrorKind::WouldBlock, "libcurl again")),
        _ => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("libcurl socket I/O failed: {rc}"),
        )),
    }
}
