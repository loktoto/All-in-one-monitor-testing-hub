//! A small opt-in TCP connect diagnostic to a user-supplied numeric address.
use std::{
    net::{SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
const ATTEMPTS: usize = 4;
#[derive(Clone, Default)]
pub struct State {
    pub running: bool,
    pub attempted: usize,
    pub succeeded: usize,
    pub total_ms: f64,
    pub last_error: Option<String>,
    pub stopped: bool,
}
pub fn start(
    target: &str,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let address: SocketAddr = target
        .trim()
        .parse()
        .map_err(|_| "Enter a numeric IP address and port, e.g. 127.0.0.1:80 or [::1]:80.")?;
    if address.port() == 0 {
        return Err("Port must be between 1 and 65535.".into());
    }
    {
        let mut s = state
            .lock()
            .map_err(|_| "Cannot access diagnostic state.")?;
        if s.running {
            return Err("A connection check is already running.".into());
        }
        *s = State {
            running: true,
            ..Default::default()
        };
    }
    cancel.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        for _ in 0..ATTEMPTS {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let started = Instant::now();
            let result = TcpStream::connect_timeout(&address, Duration::from_millis(750));
            if let Ok(mut s) = state.lock() {
                s.attempted += 1;
                match result {
                    Ok(stream) => {
                        drop(stream);
                        s.succeeded += 1;
                        s.total_ms += started.elapsed().as_secs_f64() * 1000.0;
                    }
                    Err(e) => s.last_error = Some(e.to_string()),
                }
            }
        }
        if let Ok(mut s) = state.lock() {
            s.running = false;
            s.stopped = cancel.load(Ordering::Relaxed);
        }
    });
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connects_to_a_local_listener_and_rejects_invalid_targets() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = listener.local_addr().unwrap().to_string();
        let receiver = std::thread::spawn(move || {
            for _ in 0..ATTEMPTS {
                listener.accept().unwrap();
            }
        });
        let state = Arc::new(Mutex::new(State::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(start("example.com:80", state.clone(), cancel.clone()).is_err());
        start(&target, state.clone(), cancel).unwrap();
        for _ in 0..100 {
            if !state.lock().unwrap().running {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let snapshot = state.lock().unwrap().clone();
        assert!(!snapshot.running);
        assert_eq!(snapshot.succeeded, ATTEMPTS);
        receiver.join().unwrap();
    }
}
