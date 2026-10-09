//! Bound the real UDP receive while checking the reported sender (T1650).
use super::*;
use std::time::Duration;

struct TestUdp(i64);
impl TestUdp {
    fn new() -> Self {
        let fd = udp_bind(0);
        assert!(fd > 0, "bind UDP fixture: {fd}");
        Self(fd)
    }

    fn port_with_deadline(&self) -> u16 {
        let resources = REGISTRY.lock().unwrap();
        let Some(NetResource::Udp(socket)) = resources.get(&self.0) else {
            panic!("UDP fixture socket missing");
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket.local_addr().unwrap().port()
    }
}
impl Drop for TestUdp {
    fn drop(&mut self) {
        udp_close(self.0);
    }
}

#[test]
fn udp_recv_from_returns_peer_address() {
    let receiver = TestUdp::new();
    let receive_port = receiver.port_with_deadline();
    let sender = TestUdp::new();
    let send_port = sender.port_with_deadline();
    assert_eq!(
        udp_send(sender.0, b"ping", "127.0.0.1", receive_port as i64),
        4
    );
    let received = udp_recv_from(receiver.0, 64)
        .expect("UDP fixture receive must complete within its socket deadline");
    assert_eq!(received.0, "ping");
    let (family, host, port) = received.1.expect("peer reported");
    assert_eq!(family, 4);
    assert_eq!(port, send_port as i64);
    assert!(
        host == "127.0.0.1" || host == "0.0.0.0",
        "peer host: {host}"
    );
}
