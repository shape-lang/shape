//! TCP transport implementation with length-prefixed framing.
//!
//! Uses a simple 4-byte big-endian length header followed by the payload.
//! Maximum payload size is 64 MB to prevent accidental memory exhaustion.

use super::{Connection, Transport, TransportError};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Maximum payload size: 64 MB.
pub const MAX_PAYLOAD_SIZE: usize = 64 * 1024 * 1024;

/// TCP-based transport using length-prefixed framing.
pub struct TcpTransport {
    /// Timeout for establishing new connections.
    pub connect_timeout: Duration,
    /// Read timeout applied to connections created by [`Transport::send`].
    pub read_timeout: Option<Duration>,
}

impl Default for TcpTransport {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Some(Duration::from_secs(30)),
        }
    }
}

impl Transport for TcpTransport {
    fn send(&self, destination: &str, payload: &[u8]) -> Result<Vec<u8>, TransportError> {
        if payload.len() > MAX_PAYLOAD_SIZE {
            return Err(TransportError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_PAYLOAD_SIZE,
            });
        }

        let mut stream = TcpStream::connect_timeout(
            &destination
                .parse()
                .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?,
            self.connect_timeout,
        )
        .map_err(|e| TransportError::ConnectionFailed(format!("{}: {}", destination, e)))?;

        stream.set_read_timeout(self.read_timeout).ok();
        stream.set_write_timeout(Some(self.connect_timeout)).ok();

        write_length_prefixed(&mut stream, payload)?;
        read_length_prefixed(&mut stream)
    }

    fn connect(&self, destination: &str) -> Result<Box<dyn Connection>, TransportError> {
        let stream = TcpStream::connect_timeout(
            &destination
                .parse()
                .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?,
            self.connect_timeout,
        )
        .map_err(|e| TransportError::ConnectionFailed(format!("{}: {}", destination, e)))?;

        Ok(Box::new(TcpConnection {
            stream,
            max_payload: MAX_PAYLOAD_SIZE,
        }))
    }
}

/// A persistent TCP connection with length-prefixed framing.
pub struct TcpConnection {
    stream: TcpStream,
    max_payload: usize,
}

impl TcpConnection {
    /// Wrap an already-connected `TcpStream` into a `TcpConnection`.
    pub fn from_stream(stream: TcpStream) -> Self {
        Self {
            stream,
            max_payload: MAX_PAYLOAD_SIZE,
        }
    }
}

impl Connection for TcpConnection {
    fn send(&mut self, payload: &[u8]) -> Result<(), TransportError> {
        if payload.len() > self.max_payload {
            return Err(TransportError::PayloadTooLarge {
                size: payload.len(),
                max: self.max_payload,
            });
        }
        write_length_prefixed(&mut self.stream, payload)
    }

    fn recv(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError> {
        self.stream
            .set_read_timeout(timeout)
            .map_err(TransportError::Io)?;
        read_length_prefixed(&mut self.stream)
    }

    fn close(&mut self) -> Result<(), TransportError> {
        self.stream
            .shutdown(std::net::Shutdown::Both)
            .map_err(TransportError::Io)
    }
}

// ---------------------------------------------------------------------------
// Length-prefixed framing helpers
// ---------------------------------------------------------------------------

/// Write a length-prefixed frame: compress, then write 4-byte BE length + framed payload.
pub fn write_length_prefixed(stream: &mut TcpStream, data: &[u8]) -> Result<(), TransportError> {
    let framed = super::framing::encode_framed(data);
    let len = framed.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .map_err(|e| TransportError::SendFailed(format!("write frame length: {}", e)))?;
    stream
        .write_all(&framed)
        .map_err(|e| TransportError::SendFailed(format!("write frame payload: {}", e)))?;
    stream
        .flush()
        .map_err(|e| TransportError::SendFailed(format!("flush: {}", e)))?;
    Ok(())
}

/// Read a length-prefixed frame: read raw bytes, then decompress.
pub fn read_length_prefixed(stream: &mut TcpStream) -> Result<Vec<u8>, TransportError> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|e| TransportError::ReceiveFailed(format!("read frame length: {}", e)))?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_PAYLOAD_SIZE {
        return Err(TransportError::PayloadTooLarge {
            size: len,
            max: MAX_PAYLOAD_SIZE,
        });
    }

    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .map_err(|e| TransportError::ReceiveFailed(format!("read frame payload: {}", e)))?;
    super::framing::decode_framed(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn test_length_prefixed_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let payload = b"hello transport";

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let data = read_length_prefixed(&mut conn).unwrap();
            write_length_prefixed(&mut conn, &data).unwrap();
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        write_length_prefixed(&mut stream, payload).unwrap();
        let response = read_length_prefixed(&mut stream).unwrap();

        assert_eq!(&response, payload);
        server.join().unwrap();
    }

    #[test]
    fn test_tcp_transport_one_shot() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let data = read_length_prefixed(&mut conn).unwrap();
            let mut response = b"reply:".to_vec();
            response.extend_from_slice(&data);
            write_length_prefixed(&mut conn, &response).unwrap();
        });

        let transport = TcpTransport::default();
        let result = transport.send(&addr.to_string(), b"ping").unwrap();
        assert_eq!(&result, b"reply:ping");
        server.join().unwrap();
    }

    #[test]
    fn test_tcp_connection_send_recv() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let data = read_length_prefixed(&mut conn).unwrap();
            write_length_prefixed(&mut conn, &data).unwrap();
        });

        let transport = TcpTransport::default();
        let mut conn = transport.connect(&addr.to_string()).unwrap();

        conn.send(b"test data").unwrap();
        let response = conn.recv(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(&response, b"test data");

        conn.close().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn test_payload_too_large() {
        let transport = TcpTransport::default();
        let huge = vec![0u8; MAX_PAYLOAD_SIZE + 1];
        let result = transport.send("127.0.0.1:1", &huge);
        assert!(matches!(
            result,
            Err(TransportError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn test_connection_refused() {
        let transport = TcpTransport {
            connect_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        let result = transport.connect("127.0.0.1:1");
        assert!(result.is_err());
    }
}
