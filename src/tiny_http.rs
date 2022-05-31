use core::fmt::Write;

use smoltcp::{iface::SocketHandle, socket::TcpSocket, time::Instant};

/// Not really an http client.
/// It will ignore the HTTP return code and just assume everything is fine.
/// It won't reuse a connection, ignore the content-length and all other headers sent by the server.
/// It doesn't send any additional header and only supports simple GET requests.
pub struct HttpClient<'a> {
    interface: esp_wifi::wifi_interface::Wifi<'a>,
    current_millis_fn: fn() -> u32,
}

impl<'a> HttpClient<'a> {
    pub fn new(
        interface: esp_wifi::wifi_interface::Wifi<'a>,
        current_millis_fn: fn() -> u32,
    ) -> HttpClient {
        HttpClient {
            interface,
            current_millis_fn,
        }
    }

    pub fn get(
        mut self,
        addr: smoltcp::wire::Ipv4Address,
        port: u16,
        path: &'a str,
        host: &'a str,
    ) -> HttpResponse<'a> {
        let (http_socket_handle, _) = self
            .interface
            .network_interface()
            .sockets_mut()
            .next()
            .unwrap();
        let (socket, cx) = self
            .interface
            .network_interface()
            .get_socket_and_context::<TcpSocket>(http_socket_handle);

        let remote_endpoint = (addr, port);
        socket.connect(cx, remote_endpoint, 41000).unwrap();

        HttpResponse {
            interface: self.interface,
            socket: http_socket_handle,
            stage: Stage::Connected,
            path,
            host,
            current_millis_fn: self.current_millis_fn,
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum ReceiveStage {
    Headers,
    Body,
}

#[derive(Debug, Copy, Clone)]
enum Stage {
    Connected,
    Receiving(ReceiveStage),
    Done,
    Finished,
}

#[derive(Debug, Copy, Clone)]
pub enum PollResult {
    None,
    Data(Buffer<1024>),
    Done,
    Err,
}

pub struct HttpResponse<'a> {
    interface: esp_wifi::wifi_interface::Wifi<'a>,
    socket: SocketHandle,
    stage: Stage,
    path: &'a str,
    host: &'a str,
    current_millis_fn: fn() -> u32,
}

impl<'a> HttpResponse<'a> {
    pub fn poll(&mut self) -> PollResult {
        loop {
            let res = self
                .interface
                .network_interface()
                .poll(Instant::from_millis((self.current_millis_fn)()));

            if let Ok(false) = res {
                break;
            }
        }

        match self.stage {
            Stage::Connected => {
                let socket = self
                    .interface
                    .network_interface()
                    .get_socket::<TcpSocket>(self.socket);

                let mut to_send = Buffer::<1024>::new();
                write!(
                    to_send,
                    "GET {} HTTP/1.0\r\nHost: {}\r\n\r\n",
                    self.path, self.host
                )
                .unwrap();

                if socket.send_slice(to_send.slice()).is_ok() {
                    self.stage = Stage::Receiving(ReceiveStage::Headers);
                }

                PollResult::None
            }
            Stage::Receiving(rcv_state) => {
                let mut raw = self.receive_raw();

                if let PollResult::Data(ref mut raw) = raw {
                    match rcv_state {
                        ReceiveStage::Headers => {
                            while let Some(line) = raw.next_line() {
                                if line == "" {
                                    self.stage = Stage::Receiving(ReceiveStage::Body);
                                    break;
                                }
                            }

                            PollResult::Data(Buffer::new_from_slice(raw.remaining_slice()))
                        }
                        ReceiveStage::Body => PollResult::Data(*raw),
                    }
                } else {
                    raw
                }
            }
            Stage::Done => {
                let socket = self
                    .interface
                    .network_interface()
                    .get_socket::<TcpSocket>(self.socket);
                socket.abort();

                // make sure the socket is completely closed
                loop {
                    self.interface
                        .network_interface()
                        .poll(Instant::from_millis((self.current_millis_fn)()))
                        .unwrap();

                    let socket = self
                        .interface
                        .network_interface()
                        .get_socket::<TcpSocket>(self.socket);

                    if socket.state() == smoltcp::socket::TcpState::Closed {
                        break;
                    }
                }

                self.stage = Stage::Finished;
                PollResult::None
            }
            Stage::Finished => PollResult::Done,
        }
    }

    fn receive_raw(&mut self) -> PollResult {
        let socket = self
            .interface
            .network_interface()
            .get_socket::<TcpSocket>(self.socket);

        let mut buffer = Buffer::new();
        let mut bytes = [0u8; 1024];
        if let Ok(s) = socket.recv_slice(&mut bytes) {
            if s > 0 {
                buffer.push(&bytes[..s]);
                PollResult::Data(buffer)
            } else {
                PollResult::None
            }
        } else {
            self.stage = Stage::Done;
            PollResult::None
        }
    }

    pub fn finalize(self) -> HttpClient<'a> {
        HttpClient::new(self.interface, self.current_millis_fn)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Buffer<const C: usize> {
    data: [u8; C],
    len: usize,
    write_index: usize,
    read_index: usize,
}

impl<const C: usize> Buffer<C> {
    pub fn new() -> Buffer<C> {
        Buffer {
            data: [0u8; C],
            len: 0,
            write_index: 0,
            read_index: 0,
        }
    }

    pub fn new_from_slice(slice: &[u8]) -> Buffer<C> {
        let mut res = Buffer {
            data: [0u8; C],
            len: slice.len(),
            write_index: 0,
            read_index: 0,
        };

        res.data[..slice.len()].copy_from_slice(slice);
        res
    }

    pub fn slice(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn remaining_slice(&self) -> &[u8] {
        &self.data[self.read_index..self.len]
    }

    pub fn push(&mut self, bytes: &[u8]) -> usize {
        let fitting = usize::min(bytes.len(), C - self.write_index);
        self.data[self.write_index..][..fitting].copy_from_slice(&bytes[..fitting]);
        self.len += fitting;
        self.write_index += fitting;

        fitting
    }

    pub fn is_full(&self) -> bool {
        self.len == C
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.write_index = 0;
        self.read_index = 0
    }

    pub fn split_right(self, index: usize) -> Buffer<C> {
        Buffer::new_from_slice(&self.data[index..self.len])
    }

    pub fn next_line(&mut self) -> Option<&str> {
        let mut found: Option<usize> = None;

        for idx in self.read_index..self.len {
            if self.data[idx] == b'\n' {
                found = Some(idx);
                break;
            }
        }

        if let Some(end) = found {
            let str_end = if self.data[end - 1] == b'\r' {
                end - 1
            } else {
                end
            };
            let res =
                unsafe { core::str::from_utf8_unchecked(&self.data[self.read_index..str_end]) };
            self.read_index = end + 1;
            Some(res)
        } else {
            None
        }
    }
}

impl<const C: usize> Write for Buffer<C> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        // TODO check!
        self.push(bytes);
        Ok(())
    }
}
