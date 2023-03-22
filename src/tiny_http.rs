use core::{fmt::Display, marker::PhantomData};

use embedded_io::{
    blocking::{Read, Write},
    Io,
};

#[derive(Debug, Clone, Copy)]
pub enum HttpError {
    IoError,
}

impl Display for HttpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Error").ok();
        Ok(())
    }
}

impl<E> From<E> for HttpError
where
    E: embedded_io::Error,
{
    fn from(_: E) -> Self {
        HttpError::IoError
    }
}

pub struct Response<'a, 'c, T, E, const HS: usize, const HFS: usize>
where
    T: Read + Write,
    T: Io<Error = E>,
    E: embedded_io::Error,
{
    pub code: u16,
    pub content_length: u32,
    pub content_type: HttpString<32>,
    pub headers: [HttpString<HFS>; HS],
    client: &'a mut HttpClient<'c, T, E>,
}

impl<'a, 'c, T, E, const HS: usize, const HFS: usize> Io for Response<'a, 'c, T, E, HS, HFS>
where
    T: Read + Write,
    T: Io<Error = E>,
    E: embedded_io::Error,
{
    type Error = E;
}

impl<'a, 'c, T, E, const HS: usize, const HFS: usize> Response<'a, 'c, T, E, HS, HFS>
where
    T: Read + Write,
    T: Io<Error = E>,
    E: embedded_io::Error,
{
    pub fn finish(self) {}
}

impl<'a, 'c, T, E, const HS: usize, const HFS: usize> Read for Response<'a, 'c, T, E, HS, HFS>
where
    T: Read + Write,
    T: Io<Error = E>,
    E: embedded_io::Error,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.client.stream.read(buf)
    }
}

pub struct HttpClient<'a, T, E>
where
    T: Read + Write,
    T: Io<Error = E>,
    E: embedded_io::Error,
{
    stream: T,
    host: &'a str,
    phantom: PhantomData<E>,
}

impl<'a, T, E> HttpClient<'a, T, E>
where
    T: Read + Write,
    T: Io<Error = E>,
    E: embedded_io::Error,
{
    pub fn new(host: &'a str, stream: T) -> HttpClient<'a, T, E> {
        HttpClient {
            stream,
            host,
            phantom: Default::default(),
        }
    }

    pub fn get<'r, const HS: usize, const HFS: usize>(
        &'r mut self,
        path: &'a str,
        headers: Option<&'r [HeaderField<'r>]>,
        wanted_headers: Option<&'r [&str; HS]>,
    ) -> Result<Response<'r, 'a, T, E, HS, HFS>, HttpError>
    where
        'a: 'r,
    {
        self.stream.write(b"GET ")?;
        self.stream.write(path.as_bytes())?;
        self.stream.write(b" HTTP/1.0\r\n")?;

        self.stream.write(b"Host: ")?;
        self.stream.write(self.host.as_bytes())?;
        self.stream.write(b"\r\n")?;

        match headers {
            Some(headers) => {
                for header in headers {
                    self.stream.write(header.name.as_bytes())?;
                    self.stream.write(b": ")?;
                    self.stream.write(header.value.as_bytes())?;
                    self.stream.write(b"\r\n")?;
                }
            }
            None => (),
        }

        self.stream.write(b"\r\n\r\n")?;

        self.stream.flush()?;

        let line = Line::read_line(&mut self.stream);
        let mut split = line.as_str().split(" ");
        split.next();
        let code = split.next().unwrap();
        let code = u16::from_str_radix(code, 10).unwrap();

        let mut response_headers = [HttpString::default(); HS];
        let mut content_length = 0;
        let mut content_type = HttpString::default();
        // read all the headers
        loop {
            let line = Line::read_line(&mut self.stream);
            if line.as_str() == "" {
                break;
            } else {
                let mut line = line.as_str().split(":");
                let name = line.next().unwrap().trim();
                let content = line.next().unwrap().trim();
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = u32::from_str_radix(content, 10).unwrap();
                } else if name.eq_ignore_ascii_case("content-type") {
                    content_type = HttpString::from_bytes(content.as_bytes());
                } else {
                    if let Some(wanted_headers) = wanted_headers {
                        for (i, header_name) in wanted_headers.iter().enumerate() {
                            if name.eq_ignore_ascii_case(*header_name) {
                                response_headers[i] = HttpString::from_bytes(&content.as_bytes());
                            }
                        }
                    }
                }
            }
        }

        let response = Response {
            code,
            content_length,
            content_type,
            headers: response_headers,
            client: self,
        };
        Ok(response)
    }

    pub fn finish(self) -> T {
        self.stream
    }
}

struct Line<'a> {
    data: [u8; 1024], // longer headers will get cut
    len: usize,
    _phantom: &'a (),
}

impl<'a> Line<'a> {
    pub fn read_line(read: &mut impl Read) -> Line<'a> {
        let mut line = Line {
            data: [0u8; 1024],
            len: 0,
            _phantom: &(),
        };

        let mut idx = 0;
        loop {
            let len = read.read(&mut line.data[idx..][..1]).unwrap();

            if len == 0 {
                continue;
            }

            if line.data[idx] == b'\n' {
                break;
            }

            if idx < 1023 {
                idx += 1;
            }
        }

        line.len = idx;

        if line.data[idx - 1] == b'\r' {
            line.len = idx - 1;
        }

        line
    }

    pub fn as_str(&'a self) -> &'a str {
        unsafe { core::str::from_utf8_unchecked(&self.data[..self.len]) }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct HeaderField<'a> {
    name: &'a str,
    value: &'a str,
}

#[allow(unused)]
impl<'a> HeaderField<'a> {
    pub fn new(name: &'a str, value: &'a str) -> HeaderField<'a> {
        HeaderField { name, value }
    }
}

#[derive(Clone, Copy)]
pub struct HttpString<const S: usize> {
    data: [u8; S],
    len: usize,
}

impl<const S: usize> HttpString<S> {
    pub fn from_bytes(bytes: &[u8]) -> HttpString<S> {
        let mut data = [0u8; S];
        let len = usize::min(bytes.len(), S);
        data[..len].copy_from_slice(&bytes[..len]);
        HttpString { data, len }
    }

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.data[..self.len]) }
    }
}

impl<const S: usize> Display for HttpString<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str()).ok();
        Ok(())
    }
}

impl<const S: usize> Default for HttpString<S> {
    fn default() -> Self {
        Self::from_bytes(&[])
    }
}

impl<const S: usize> core::fmt::Debug for HttpString<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpString")
            .field("str", &self.as_str())
            .finish()
    }
}
