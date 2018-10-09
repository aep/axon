extern crate nix;
extern crate tokio;
extern crate mio;

use tokio::io::{AsyncRead, AsyncWrite};
use nix::sys::socket::socketpair;
use nix::sys::socket::AddressFamily;
use nix::sys::socket::SockType;
use nix::sys::socket::SockFlag;
use nix::unistd::dup2;
use std::os::unix::io::RawFd;
use std::io::Error;
use std::io::ErrorKind;
use std::io;
use std::net::Shutdown;
use tokio::prelude::Async;
use std::process::Child;
use std::os::unix::process::CommandExt as StdUnixCommandExt;
use std::io::{Read,Write};
use std::env;
use std::process::Command;
use tokio::reactor::PollEvented2;
use nix::unistd::close;
use nix::unistd::{read, write};
use nix::unistd::fsync;
use std::mem::transmute;

pub trait CommandExt {
    fn spawn_with_axon(&mut self) -> io::Result<(Child, Io)>;
}

impl CommandExt for Command {
    fn spawn_with_axon(&mut self) -> io::Result<(Child, Io)> {
        let (i1, i2) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::empty(),
        ).unwrap();

        let (o1, o2) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::empty(),
        ).unwrap();


        self.env("AXON_FD_IN", "4");
        self.env("AXON_FD_OUT", "5");

        self.before_exec(move || {
            close(i1).unwrap();
            close(o1).unwrap();


            dup2(i2, 4.into())
                .map_err(|e| Error::new(ErrorKind::Other, e))
                ?;
            dup2(o2, 5.into())
                .map_err(|e| Error::new(ErrorKind::Other, e))
                ?;
            Ok(())
        });

        let child = self.spawn()?;

        close(i2).unwrap();
        close(o2).unwrap();

        let io = Io {
            axon_in : o1,
            axon_out: i1,
            stream:   false,
        };


        Ok((child, io))
    }

}


pub struct Io {
    axon_in:  RawFd,
    axon_out: RawFd,
    stream:   bool,
}


impl mio::event::Evented for Io {
    fn register(&self, poll: &mio::Poll, token: mio::Token, interest: mio::Ready, opts: mio::PollOpt) -> io::Result<()> {
        mio::unix::EventedFd(&self.axon_in).register(poll, token, interest, opts)
    }

    fn reregister(&self, poll: &mio::Poll, token: mio::Token, interest: mio::Ready, opts: mio::PollOpt) -> io::Result<()> {
        mio::unix::EventedFd(&self.axon_in).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
        mio::unix::EventedFd(&self.axon_in).deregister(poll)
    }
}

impl Io {
    pub fn into_async(self, handle: &tokio::reactor::Handle) -> io::Result<PollEvented2<Self>> {
        use nix::fcntl::{fcntl, FdFlag, OFlag};
        use nix::fcntl::FcntlArg::{F_SETFD, F_SETFL};

        fcntl(self.axon_in, F_SETFD(FdFlag::FD_CLOEXEC))
            .map_err(|e| Error::new(ErrorKind::Other, e))
            ?;
        fcntl(self.axon_in, F_SETFL(OFlag::O_NONBLOCK))
            .map_err(|e| Error::new(ErrorKind::Other, e))
            ?;

        PollEvented2::new_with_handle(self, handle)
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match how {
            Shutdown::Read => {
                close(self.axon_in).ok();
            },
            Shutdown::Write => {
                close(self.axon_out).ok();
            },
            Shutdown::Both => {
                close(self.axon_in).ok();
                close(self.axon_out).ok();
            },
        }
        Ok(())
    }
}

impl Read for Io {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match read(self.axon_in, buf) {
            Ok(v) => Ok(v),
            Err(nix::Error::Sys(errno)) => Err(io::Error::from_raw_os_error(errno as i32)),
            Err(e) => Err(Error::new(ErrorKind::Other, e)),
        }
    }
}

impl Write for Io {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.stream {
            let len : [u8; 8] = unsafe {transmute(buf.len())};
            write(self.axon_out, &len).ok();
            write(self.axon_out, b"\n").ok();
        }
        let r = match write(self.axon_out, buf) {
            Ok(v) => Ok(v),
            Err(nix::Error::Sys(errno)) => Err(io::Error::from_raw_os_error(errno as i32)),
            Err(e) => Err(Error::new(ErrorKind::Other, e)),
        };

        if self.stream {
            write(self.axon_out, b"\n").ok();
        }

        r
    }

    fn flush(&mut self) -> io::Result<()> {
        fsync(self.axon_out).ok();
        Ok(())
    }
}


impl AsyncRead  for Io {}
impl AsyncWrite for Io {
    fn shutdown(&mut self) -> Result<Async<()>, tokio::io::Error> {
        Io::shutdown(self, Shutdown::Both)?;
        Ok(Async::Ready(()))
    }
}



fn from_axion() -> Result<Io, Error> {
    let axon_out : RawFd = env::var("AXON_FD_OUT")
        .map_err(|_| Error::new(ErrorKind::Other, format!("AXON_FD_OUT missing. executable needs to be spawned from an axon host")))?
        .parse()
        .map_err(|e| Error::new(ErrorKind::Other, e))?;

    let axon_in : RawFd = env::var("AXON_FD_IN")
        .map_err(|_| Error::new(ErrorKind::Other, format!("AXON_FD_IN  missing. executable needs to be spawned from an axon host")))?
        .parse()
        .map_err(|e| Error::new(ErrorKind::Other, e))?;

    Ok(Io {
        axon_in,
        axon_out,
        stream: false,
    })
}

pub fn from_std() -> Io {
    Io {
        axon_in:  0.into(),
        axon_out: 1.into(),
        stream: true,
    }
}

pub fn child() -> Io {
    match from_axion() {
        Ok(v)  => return v,
        Err(e) => eprintln!("{}", e),
    }
    from_std()
}


