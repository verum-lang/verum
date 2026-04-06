// Network Cross-Platform Tests
// Validates networking across all platforms

use super::detection::PlatformInfo;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct NetworkTestHarness {
    pub platform: PlatformInfo,
}

impl NetworkTestHarness {
    pub fn new() -> Self {
        Self {
            platform: PlatformInfo::detect(),
        }
    }

    /// Get an available port for testing
    pub fn get_available_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_socket_basic() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        // Server thread
        let server_addr = addr.clone();
        let server = thread::spawn(move || {
            let listener = TcpListener::bind(&server_addr).unwrap();
            let (mut stream, _) = listener.accept().unwrap();

            let mut buffer = [0u8; 128];
            let n = stream.read(&mut buffer).unwrap();
            let received = String::from_utf8_lossy(&buffer[..n]);

            stream.write_all(b"pong").unwrap();
            received.to_string()
        });

        // Give server time to start
        thread::sleep(Duration::from_millis(100));

        // Client
        let mut stream = TcpStream::connect(&addr).unwrap();
        stream.write_all(b"ping").unwrap();

        let mut buffer = [0u8; 128];
        let n = stream.read(&mut buffer).unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);

        assert_eq!(response, "pong");

        let received = server.join().unwrap();
        assert_eq!(received, "ping");
    }

    #[test]
    fn test_udp_socket() {
        let port1 = NetworkTestHarness::get_available_port();
        let port2 = NetworkTestHarness::get_available_port();

        let addr1 = format!("127.0.0.1:{}", port1);
        let addr2 = format!("127.0.0.1:{}", port2);

        let socket1 = UdpSocket::bind(&addr1).unwrap();
        let socket2 = UdpSocket::bind(&addr2).unwrap();

        // Send from socket1 to socket2
        socket1.send_to(b"hello udp", &addr2).unwrap();

        // Receive on socket2
        let mut buffer = [0u8; 128];
        socket2.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (n, src) = socket2.recv_from(&mut buffer).unwrap();

        let message = String::from_utf8_lossy(&buffer[..n]);
        assert_eq!(message, "hello udp");
        assert_eq!(src.port(), port1);
    }

    #[test]
    fn test_tcp_concurrent_connections() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        // Server thread
        let server_addr = addr.clone();
        let server = thread::spawn(move || {
            let listener = TcpListener::bind(&server_addr).unwrap();

            for _ in 0..10 {
                let (mut stream, _) = listener.accept().unwrap();
                let counter = Arc::clone(&counter_clone);

                thread::spawn(move || {
                    let mut buffer = [0u8; 128];
                    if stream.read(&mut buffer).is_ok() {
                        counter.fetch_add(1, Ordering::SeqCst);
                        stream.write_all(b"ack").unwrap();
                    }
                });
            }
        });

        thread::sleep(Duration::from_millis(100));

        // Spawn 10 clients
        let mut client_handles = Vec::new();
        for i in 0..10 {
            let addr = addr.clone();
            let handle = thread::spawn(move || {
                let mut stream = TcpStream::connect(&addr).unwrap();
                stream.write_all(format!("client {}", i).as_bytes()).unwrap();

                let mut buffer = [0u8; 128];
                stream.read(&mut buffer).unwrap();
            });
            client_handles.push(handle);
        }

        for handle in client_handles {
            handle.join().unwrap();
        }

        server.join().unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_socket_options() {
        let port = NetworkTestHarness::get_available_port();
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).unwrap();

        // SO_REUSEADDR
        #[cfg(not(windows))]
        {
            use std::os::unix::io::AsRawFd;
            let fd = listener.as_raw_fd();
            let mut optval: libc::c_int = 1;
            let result = unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_REUSEADDR,
                    &mut optval as *mut _ as *mut libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            assert_eq!(result, 0);
        }

        #[cfg(windows)]
        {
            // Windows socket options
            println!("Windows socket options test");
        }
    }

    #[test]
    fn test_nonblocking_socket() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        let listener = TcpListener::bind(&addr).unwrap();
        listener.set_nonblocking(true).unwrap();

        // Try to accept (should fail with WouldBlock)
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Expected
            }
            other => panic!("Expected WouldBlock, got {:?}", other),
        }

        // Spawn client
        let addr_clone = addr.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            TcpStream::connect(&addr_clone).unwrap();
        });

        // Poll for connection
        for _ in 0..100 {
            match listener.accept() {
                Ok((stream, _)) => {
                    println!("Accepted connection");
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("Unexpected error: {}", e),
            }
        }

        panic!("No connection accepted");
    }

    #[test]
    fn test_socket_timeout() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        let listener = TcpListener::bind(&addr).unwrap();

        // Spawn client that connects but doesn't send data
        let addr_clone = addr.clone();
        thread::spawn(move || {
            let _stream = TcpStream::connect(&addr_clone).unwrap();
            thread::sleep(Duration::from_secs(10));
        });

        let (mut stream, _) = listener.accept().unwrap();

        // Set read timeout
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();

        let mut buffer = [0u8; 128];
        let result = stream.read(&mut buffer);

        // Should timeout
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn test_ipv6_socket() {
        // Try to bind IPv6
        match TcpListener::bind("[::1]:0") {
            Ok(listener) => {
                println!("IPv6 supported");
                let addr = listener.local_addr().unwrap();
                assert!(addr.is_ipv6());

                // Test IPv6 connection
                let port = addr.port();
                let client = TcpStream::connect(format!("[::1]:{}", port));
                if client.is_ok() {
                    println!("IPv6 connection successful");
                }
            }
            Err(e) => {
                println!("IPv6 not available: {}", e);
            }
        }
    }

    #[test]
    fn test_tcp_nodelay() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        let listener = TcpListener::bind(&addr).unwrap();

        thread::spawn(move || {
            TcpStream::connect(&addr).unwrap();
        });

        let (stream, _) = listener.accept().unwrap();

        // Enable TCP_NODELAY (disable Nagle's algorithm)
        stream.set_nodelay(true).unwrap();
        assert!(stream.nodelay().unwrap());

        stream.set_nodelay(false).unwrap();
        assert!(!stream.nodelay().unwrap());
    }

    #[test]
    fn test_keepalive() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        let listener = TcpListener::bind(&addr).unwrap();

        thread::spawn(move || {
            TcpStream::connect(&addr).unwrap();
        });

        let (stream, _) = listener.accept().unwrap();

        // Set keepalive
        #[cfg(not(windows))]
        {
            use std::os::unix::io::AsRawFd;
            let fd = stream.as_raw_fd();
            let mut optval: libc::c_int = 1;
            unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_KEEPALIVE,
                    &mut optval as *mut _ as *mut libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        #[cfg(windows)]
        {
            println!("Windows keepalive test");
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_unix_domain_socket() {
        let socket_path = std::env::temp_dir().join(format!("test_socket_{}", std::process::id()));

        // Remove if exists
        let _ = std::fs::remove_file(&socket_path);

        use std::os::unix::net::{UnixListener, UnixStream};

        let listener = UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        let client_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let mut stream = UnixStream::connect(&socket_path_clone).unwrap();
            stream.write_all(b"unix socket test").unwrap();
        });

        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 128];
        let n = stream.read(&mut buffer).unwrap();

        assert_eq!(&buffer[..n], b"unix socket test");

        client_thread.join().unwrap();
        std::fs::remove_file(&socket_path).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn test_windows_named_pipes() {
        use std::os::windows::io::AsRawHandle;

        // Windows named pipes are created differently
        // This is a placeholder for named pipe testing
        println!("Windows named pipe test");

        // Named pipes in Windows use CreateNamedPipe API
        // For basic testing, we verify pipe-like behavior through TCP
        let port = NetworkTestHarness::get_available_port();
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).unwrap();

        let handle = listener.as_raw_handle();
        assert!(!handle.is_null());
    }

    #[test]
    fn test_socket_buffer_sizes() {
        let port = NetworkTestHarness::get_available_port();
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).unwrap();

        thread::spawn(move || {
            TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        });

        let (stream, _) = listener.accept().unwrap();

        // Get/set send buffer size
        #[cfg(not(windows))]
        {
            use std::os::unix::io::AsRawFd;
            let fd = stream.as_raw_fd();

            // Get current send buffer size
            let mut optval: libc::c_int = 0;
            let mut optlen: libc::socklen_t = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

            unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    &mut optval as *mut _ as *mut libc::c_void,
                    &mut optlen,
                );
            }

            println!("Send buffer size: {}", optval);
            assert!(optval > 0);
        }

        #[cfg(windows)]
        {
            println!("Windows socket buffer test");
        }
    }

    #[test]
    fn test_multicast_udp() {
        // Multicast address
        let multicast_addr = "239.255.0.1:5000";

        match UdpSocket::bind("0.0.0.0:5000") {
            Ok(socket) => {
                // Join multicast group
                let multicast_ip: std::net::Ipv4Addr = "239.255.0.1".parse().unwrap();
                let interface_ip: std::net::Ipv4Addr = "0.0.0.0".parse().unwrap();

                #[cfg(not(windows))]
                {
                    use std::os::unix::io::AsRawFd;
                    let fd = socket.as_raw_fd();

                    let mreq = libc::ip_mreq {
                        imr_multiaddr: libc::in_addr {
                            s_addr: u32::from_ne_bytes(multicast_ip.octets()),
                        },
                        imr_interface: libc::in_addr {
                            s_addr: u32::from_ne_bytes(interface_ip.octets()),
                        },
                    };

                    unsafe {
                        libc::setsockopt(
                            fd,
                            libc::IPPROTO_IP,
                            libc::IP_ADD_MEMBERSHIP,
                            &mreq as *const _ as *const libc::c_void,
                            std::mem::size_of::<libc::ip_mreq>() as libc::socklen_t,
                        );
                    }
                }

                println!("Multicast test complete");
            }
            Err(e) => {
                println!("Multicast not available: {}", e);
            }
        }
    }

    #[test]
    fn test_network_error_conditions() {
        // Test connection refused
        match TcpStream::connect("127.0.0.1:1") {
            Err(e) => {
                assert!(
                    e.kind() == std::io::ErrorKind::ConnectionRefused
                        || e.kind() == std::io::ErrorKind::TimedOut
                );
            }
            Ok(_) => panic!("Connection should fail"),
        }

        // Test invalid address
        match TcpStream::connect("invalid.address.local:80") {
            Err(e) => {
                println!("Invalid address error: {}", e);
            }
            Ok(_) => println!("Unexpected success"),
        }
    }

    #[test]
    fn test_concurrent_network_operations() {
        let port = NetworkTestHarness::get_available_port();
        let addr = format!("127.0.0.1:{}", port);

        let listener = TcpListener::bind(&addr).unwrap();
        let addr_clone = addr.clone();

        thread::spawn(move || {
            for _ in 0..20 {
                if let Ok((mut stream, _)) = listener.accept() {
                    thread::spawn(move || {
                        let mut buffer = [0u8; 1024];
                        if let Ok(n) = stream.read(&mut buffer) {
                            stream.write_all(&buffer[..n]).unwrap();
                        }
                    });
                }
            }
        });

        thread::sleep(Duration::from_millis(100));

        let mut client_handles = Vec::new();
        for i in 0..20 {
            let addr = addr_clone.clone();
            let handle = thread::spawn(move || {
                let mut stream = TcpStream::connect(&addr).unwrap();
                let message = format!("Message {}", i);
                stream.write_all(message.as_bytes()).unwrap();

                let mut buffer = vec![0u8; message.len()];
                stream.read_exact(&mut buffer).unwrap();

                String::from_utf8(buffer).unwrap()
            });
            client_handles.push(handle);
        }

        for (i, handle) in client_handles.into_iter().enumerate() {
            let response = handle.join().unwrap();
            assert_eq!(response, format!("Message {}", i));
        }
    }
}
