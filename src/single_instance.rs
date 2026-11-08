//! Single instance: launching the app again brings the running window to the
//! front instead of starting a second copy (two copies would fight over gain
//! locks and auto-switching).
//!
//! The first instance listens on a per-user localhost port. A new instance
//! connects, sends `SHOW` and waits for `OK`. Without a valid reply (the port is
//! used by something else, or nothing listens) it just starts normally.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const HELLO: &str = "AUDIO-INTERFACE SHOW";
const REPLY: &str = "AUDIO-INTERFACE OK";

/// A port derived from the user name, so different users don't collide.
pub fn port() -> u16 {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    let h = user.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    40_000 + (h % 20_000) as u16
}

/// Ask a running instance to show itself. True if one answered.
pub fn signal_existing(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if writeln!(s, "{HELLO}").is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).is_ok() && line.trim() == REPLY
}

/// Become the primary instance: listen for `SHOW` requests. The returned flag
/// is set whenever another launch asks us to show the window, and `on_show`
/// is called (use it to wake the UI). `None` if the port is unavailable.
pub fn listen(port: u16, on_show: impl Fn() + Send + 'static) -> Option<Arc<AtomicBool>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()?;
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let _ = conn.set_read_timeout(Some(Duration::from_secs(1)));
                let mut reader = BufReader::new(&conn);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() && line.trim() == HELLO {
                    let _ = writeln!(&conn, "{REPLY}");
                    f.store(true, Ordering::SeqCst);
                    on_show();
                }
            }
        })
        .ok()?;
    Some(flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_launch_reaches_the_first() {
        // Use an OS-assigned free port for the test.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        assert!(!signal_existing(port), "nothing listening yet");
        let woke = Arc::new(AtomicBool::new(false));
        let w = woke.clone();
        let flag = listen(port, move || w.store(true, Ordering::SeqCst)).expect("listen");
        assert!(signal_existing(port));
        assert!(flag.load(Ordering::SeqCst));
        assert!(woke.load(Ordering::SeqCst));
        assert!(listen(port, || {}).is_none(), "second listener can't bind");
    }

    #[test]
    fn foreign_service_on_the_port_is_not_mistaken_for_us() {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let _ = writeln!(&c, "HTTP/1.1 400 Bad Request");
            }
        });
        assert!(!signal_existing(port));
    }
}
