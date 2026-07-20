use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
struct Failure {
    count: u8,
    at: Instant,
}
pub(super) struct PeerGuard {
    current: Mutex<IpAddr>,
    failures: Mutex<HashMap<IpAddr, Failure>>,
}
impl PeerGuard {
    pub(super) fn new() -> Self {
        Self {
            current: Mutex::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            failures: Mutex::new(HashMap::new()),
        }
    }
    pub(super) fn set_peer(&self, ip: IpAddr) {
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = ip;
    }
    pub(super) fn peer(&self) -> IpAddr {
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn blocked_for(&self, ip: IpAddr) -> Option<Duration> {
        let entries = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let e = entries.get(&ip)?;
        let lock = match e.count {
            0..=3 => return None,
            4 => Duration::from_secs(5),
            5 => Duration::from_secs(15),
            _ => Duration::from_secs(60),
        };
        lock.checked_sub(e.at.elapsed())
    }
    pub(super) fn failure(&self, ip: IpAddr) {
        let mut entries = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let e = entries.entry(ip).or_insert(Failure {
            count: 0,
            at: Instant::now(),
        });
        e.count = e.count.saturating_add(1);
        e.at = Instant::now();
    }
    pub(super) fn success(&self, ip: IpAddr) {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&ip);
    }
    pub(super) fn prune(&self) {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|_, e| e.at.elapsed() < Duration::from_secs(120));
    }
}
