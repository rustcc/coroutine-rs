// The MIT License (MIT)

// Copyright (c) 2015 Rustcc Developers

// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:

// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
// COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::io;
use std::net::{ToSocketAddrs, SocketAddr};
use std::ops::{Deref, DerefMut};

use mio::{self, Interest};
use mio::buf::{Buf, MutBuf, MutSliceBuf, SliceBuf};

use scheduler::Scheduler;

pub struct TcpSocket(::mio::tcp::TcpSocket);

impl TcpSocket {
    /// Returns a new, unbound, non-blocking, IPv4 socket
    pub fn v4() -> io::Result<TcpSocket> {
        Ok(TcpSocket(try!(::mio::tcp::TcpSocket::v4())))
    }

    /// Returns a new, unbound, non-blocking, IPv6 socket
    pub fn v6() -> io::Result<TcpSocket> {
        Ok(TcpSocket(try!(::mio::tcp::TcpSocket::v6())))
    }

    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<(TcpStream, bool)> {
        // let (stream, complete) = try!(self.0.connect(addr));
        // Ok((TcpStream(stream), complete))

        super::each_addr(addr, |a| {
            match a {
                &SocketAddr::V4(..) => try!(TcpSocket::v4()).0.connect(a),
                &SocketAddr::V6(..) => try!(TcpSocket::v6()).0.connect(a),
            }
        }).map(|(stream, complete)| (TcpStream(stream), complete))
    }

    pub fn listen(self, backlog: usize) -> io::Result<TcpListener> {
        Ok(TcpListener(try!(self.0.listen(backlog))))
    }
}

impl Deref for TcpSocket {
    type Target = ::mio::tcp::TcpSocket;

    fn deref(&self) -> &::mio::tcp::TcpSocket {
        &self.0
    }
}

impl DerefMut for TcpSocket {
    fn deref_mut(&mut self) -> &mut ::mio::tcp::TcpSocket {
        &mut self.0
    }
}

pub struct TcpListener(::mio::tcp::TcpListener);

impl TcpListener {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<TcpListener> {
        // let listener = try!(::mio::tcp::TcpListener::bind(addr));

        // Ok(TcpListener(listener))
        super::each_addr(addr, ::mio::tcp::TcpListener::bind).map(TcpListener)
    }

    pub fn accept(&self) -> io::Result<TcpStream> {
        match self.0.accept() {
            Ok(None) => {
                debug!("accept WouldBlock; going to register into eventloop");
            },
            Ok(Some(stream)) => {
                return Ok(TcpStream(stream));
            },
            Err(err) => {
                return Err(err);
            }
        }

        try!(Scheduler::current().wait_event(&self.0, Interest::readable()));

        match self.0.accept() {
            Ok(None) => {
                panic!("accept WouldBlock; Coroutine was awaked by readable event");
            },
            Ok(Some(stream)) => {
                Ok(TcpStream(stream))
            },
            Err(err) => {
                Err(err)
            }
        }
    }

    pub fn try_clone(&self) -> io::Result<TcpListener> {
        Ok(TcpListener(try!(self.0.try_clone())))
    }
}

impl Deref for TcpListener {
    type Target = ::mio::tcp::TcpListener;

    fn deref(&self) -> &::mio::tcp::TcpListener {
        &self.0
    }
}

impl DerefMut for TcpListener {
    fn deref_mut(&mut self) -> &mut ::mio::tcp::TcpListener {
        &mut self.0
    }
}

pub struct TcpStream(mio::tcp::TcpStream);

impl TcpStream {
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
        // let stream = try!(mio::tcp::TcpStream::connect(addr));

        // Ok(TcpStream(stream))
        super::each_addr(addr, ::mio::tcp::TcpStream::connect).map(TcpStream)
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.0.peer_addr()
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }

    pub fn try_clone(&self) -> io::Result<TcpStream> {
        let stream = try!(self.0.try_clone());

        Ok(TcpStream(stream))
    }
}

impl io::Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use mio::TryRead;

        let mut buf = MutSliceBuf::wrap(buf);
        let mut total_len = 0;
        while buf.has_remaining() {
            match self.0.read(&mut buf) {
                Ok(None) => {
                    debug!("TcpStream read WouldBlock");
                    break;
                },
                Ok(Some(0)) => {
                    debug!("TcpStream read 0 bytes; may be EOF");
                    return Ok(total_len);
                },
                Ok(Some(len)) => {
                    debug!("TcpStream read {} bytes", len);
                    total_len += len;
                },
                Err(err) => {
                    return Err(err);
                }
            }
        }

        if total_len != 0 {
            // We got something, just return!
            return Ok(total_len);
        }

        debug!("Read: Going to register event");
        try!(Scheduler::current().wait_event(&self.0, Interest::readable()));
        debug!("Read: Got read event");

        while buf.has_remaining() {
            match self.0.read(&mut buf) {
                Ok(None) => {
                    debug!("TcpStream read WouldBlock");
                    break;
                },
                Ok(Some(0)) => {
                    debug!("TcpStream read 0 bytes; may be EOF");
                    break;
                },
                Ok(Some(len)) => {
                    debug!("TcpStream read {} bytes", len);
                    total_len += len;
                },
                Err(err) => {
                    return Err(err);
                }
            }
        }

        Ok(total_len)
    }
}

impl io::Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use mio::TryWrite;

        let mut buf = SliceBuf::wrap(buf);
        let mut total_len = 0;

        while buf.has_remaining() {
            match self.0.write(&mut buf) {
                Ok(None) => {
                    debug!("TcpStream write WouldBlock");
                    break;
                },
                Ok(Some(0)) => {
                    debug!("TcpStream write 0 bytes; may be EOF");
                    return Ok(total_len);
                },
                Ok(Some(len)) => {
                    debug!("TcpStream written {} bytes", len);
                    total_len += len;
                },
                Err(err) => {
                    return Err(err)
                }
            }
        }

        if total_len != 0 {
            // We have written something, return it!
            return Ok(total_len)
        }

        debug!("Write: Going to register event");
        try!(Scheduler::current().wait_event(&self.0, Interest::writable()));
        debug!("Write: Got write event");

        while buf.has_remaining() {
            match self.0.write(&mut buf) {
                Ok(None) => {
                    debug!("TcpStream write WouldBlock");
                    break;
                },
                Ok(Some(0)) => {
                    debug!("TcpStream write 0 bytes; may be EOF");
                    break;
                },
                Ok(Some(len)) => {
                    debug!("TcpStream written {} bytes", len);
                    total_len += len;
                },
                Err(err) => {
                    return Err(err)
                }
            }
        }

        Ok(total_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Deref for TcpStream {
    type Target = ::mio::tcp::TcpStream;

    fn deref(&self) -> &::mio::tcp::TcpStream {
        &self.0
    }
}

impl DerefMut for TcpStream {
    fn deref_mut(&mut self) -> &mut ::mio::tcp::TcpStream {
        &mut self.0
    }
}
