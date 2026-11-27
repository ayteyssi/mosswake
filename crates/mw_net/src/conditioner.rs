//! [`LinkConditioner`]: a UDP proxy that adds latency, jitter and packet loss between
//! clients and a server, to test and tune netcode on a perfect LAN (or on one machine).
//!
//! Clients connect to the proxy instead of the server. Each client gets its own upstream
//! socket, so the server sees distinct addresses. Time is logical ([`LinkConditioner::update`]
//! takes `dt`) and loss is driven by a seeded generator: runs are reproducible.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// Network conditions applied in **each direction**.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conditions {
    /// Base one-way delay (a 50 ms latency gives a 100 ms round trip).
    pub latency: Duration,
    /// Extra random delay in `0..=jitter` (packets may arrive out of order, as with UDP).
    pub jitter: Duration,
    /// Fraction of packets dropped, `0.0..=1.0`.
    pub loss: f32,
    /// Seed of the loss and jitter generator.
    pub seed: u64,
}

impl Default for Conditions {
    fn default() -> Self {
        Self {
            latency: Duration::ZERO,
            jitter: Duration::ZERO,
            loss: 0.0,
            seed: 0x5EED,
        }
    }
}

/// A delayed datagram, ordered by delivery time then arrival order.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InFlight {
    due: Duration,
    seq: u64,
    /// `Some(client)`: server → client; `None`: client → server (through `upstream_of`).
    to_client: Option<SocketAddr>,
    upstream_of: SocketAddr,
    data: Vec<u8>,
}

/// Counters, for tests and logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConditionerStats {
    pub forwarded: u64,
    pub dropped: u64,
}

/// The proxy. Call [`LinkConditioner::update`] regularly (every tick or more often).
pub struct LinkConditioner {
    front: UdpSocket,
    server: SocketAddr,
    conditions: Conditions,
    upstreams: BTreeMap<SocketAddr, UdpSocket>,
    queue: BinaryHeap<Reverse<InFlight>>,
    now: Duration,
    seq: u64,
    rng: u64,
    stats: ConditionerStats,
}

impl LinkConditioner {
    /// Listens on `listen` (port 0 = any) and forwards to `server`.
    pub fn bind(
        listen: SocketAddr,
        server: SocketAddr,
        conditions: Conditions,
    ) -> io::Result<Self> {
        let front = UdpSocket::bind(listen)?;
        front.set_nonblocking(true)?;
        Ok(Self {
            front,
            server,
            conditions,
            upstreams: BTreeMap::new(),
            queue: BinaryHeap::new(),
            now: Duration::ZERO,
            seq: 0,
            rng: conditions.seed | 1,
            stats: ConditionerStats::default(),
        })
    }

    /// Address clients should connect to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.front.local_addr()
    }

    pub fn stats(&self) -> ConditionerStats {
        self.stats
    }

    /// xorshift64*: tiny, deterministic, good enough for packet fates.
    fn next_random(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn enqueue(&mut self, to_client: Option<SocketAddr>, upstream_of: SocketAddr, data: Vec<u8>) {
        if self.next_random() < f64::from(self.conditions.loss) {
            self.stats.dropped += 1;
            return;
        }
        let jitter = self.conditions.jitter.mul_f64(self.next_random());
        self.seq += 1;
        self.queue.push(Reverse(InFlight {
            due: self.now + self.conditions.latency + jitter,
            seq: self.seq,
            to_client,
            upstream_of,
            data,
        }));
    }

    /// Advances logical time by `dt`, takes in every waiting datagram and delivers those due.
    pub fn update(&mut self, dt: Duration) -> io::Result<()> {
        self.now += dt;
        let mut buffer = vec![0u8; 64 * 1024];
        // Client → proxy.
        loop {
            match self.front.recv_from(&mut buffer) {
                Ok((len, client)) => {
                    if !self.upstreams.contains_key(&client) {
                        let upstream =
                            UdpSocket::bind(SocketAddr::new(self.front.local_addr()?.ip(), 0))?;
                        upstream.set_nonblocking(true)?;
                        self.upstreams.insert(client, upstream);
                    }
                    self.enqueue(None, client, buffer[..len].to_vec());
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
        // Server → proxy (one upstream socket per client).
        let clients: Vec<SocketAddr> = self.upstreams.keys().copied().collect();
        for client in clients {
            loop {
                let received = self.upstreams[&client].recv_from(&mut buffer);
                match received {
                    Ok((len, _)) => self.enqueue(Some(client), client, buffer[..len].to_vec()),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                    Err(e) => return Err(e),
                }
            }
        }
        // Deliver what is due.
        while self
            .queue
            .peek()
            .is_some_and(|Reverse(p)| p.due <= self.now)
        {
            let Some(Reverse(packet)) = self.queue.pop() else {
                break;
            };
            let sent = match packet.to_client {
                Some(client) => self.front.send_to(&packet.data, client),
                None => self.upstreams[&packet.upstream_of].send_to(&packet.data, self.server),
            };
            match sent {
                Ok(_) => self.stats.forwarded += 1,
                // The other side went away: like the real network, just lose it.
                Err(_) => self.stats.dropped += 1,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn echo_server() -> UdpSocket {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_nonblocking(true).unwrap();
        s
    }

    fn pump_echo(server: &UdpSocket) {
        let mut buf = [0u8; 2048];
        while let Ok((len, from)) = server.recv_from(&mut buf) {
            server.send_to(&buf[..len], from).unwrap();
        }
    }

    #[test]
    fn delays_by_the_configured_latency_both_ways() {
        let server = echo_server();
        let conditions = Conditions {
            latency: Duration::from_millis(50),
            ..Conditions::default()
        };
        let mut proxy = LinkConditioner::bind(
            "127.0.0.1:0".parse().unwrap(),
            server.local_addr().unwrap(),
            conditions,
        )
        .unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.set_nonblocking(true).unwrap();
        client
            .send_to(b"ping", proxy.local_addr().unwrap())
            .unwrap();
        let step = Duration::from_millis(10);
        let mut answered_after = None;
        let mut buf = [0u8; 16];
        for i in 1..=30 {
            sleep(Duration::from_millis(1));
            proxy.update(step).unwrap();
            sleep(Duration::from_millis(1));
            pump_echo(&server);
            if client.recv_from(&mut buf).is_ok() {
                answered_after = Some(i);
                break;
            }
        }
        // 50 ms there + 50 ms back, at 10 ms steps (±1 step for the receive timing).
        let steps = answered_after.expect("echo came back");
        assert!((10..=12).contains(&steps), "round trip took {steps} steps");
    }

    #[test]
    fn loss_is_seeded_and_close_to_the_rate() {
        let server = echo_server();
        let conditions = Conditions {
            loss: 0.25,
            ..Conditions::default()
        };
        let run = || {
            let mut proxy = LinkConditioner::bind(
                "127.0.0.1:0".parse().unwrap(),
                server.local_addr().unwrap(),
                conditions,
            )
            .unwrap();
            let client = UdpSocket::bind("127.0.0.1:0").unwrap();
            // One packet at a time: a burst would overflow the kernel's socket buffer and
            // measure the OS instead of the conditioner.
            let handled = |p: &LinkConditioner| p.stats().forwarded + p.stats().dropped;
            for sent in 1..=400 {
                client.send_to(b"x", proxy.local_addr().unwrap()).unwrap();
                for _ in 0..100 {
                    proxy.update(Duration::from_millis(1)).unwrap();
                    if handled(&proxy) == sent {
                        break;
                    }
                    sleep(Duration::from_micros(50));
                }
            }
            assert_eq!(proxy.stats().forwarded + proxy.stats().dropped, 400);
            proxy.stats()
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b, "same seed, same fate");
        assert!((70..=130).contains(&a.dropped), "{a:?}");
        let mut buf = [0u8; 8];
        while server.recv_from(&mut buf).is_ok() {}
    }
}
