extern crate nix;
extern crate tokio;

use tokio::io::{AsyncRead, AsyncWrite};
use nix::sys::socket::socketpair;
use nix::sys::socket::AddressFamily;
use nix::sys::socket::SockType;
use nix::sys::socket::SockFlag;
use std::os::unix::net::UnixDatagram;
use std::os::unix::io::FromRawFd;
use nix::unistd::dup2;
use std::os::unix::io::RawFd;
use std::io::Error;
use std::io::ErrorKind;
use std::io;
use nix::unistd::fsync;
use std::os::unix::io::AsRawFd;
use std::net::Shutdown;
use tokio::prelude::Async;
use std::process::Child;
use std::os::unix::process::CommandExt as StdUnixCommandExt;
use std::io::{Read,Write};
use std::env;
use std::process::Command;

pub trait CommandExt {
    fn spawn_with_axon(&mut self) -> io::Result<(Child, AxiomIo)>;
}

impl CommandExt for Command {
    fn spawn_with_axon(&mut self) -> io::Result<(Child, AxiomIo)> {
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

        let axon_in   = unsafe{ UnixDatagram::from_raw_fd(o1) };
        let axon_out  = unsafe{ UnixDatagram::from_raw_fd(i1) };

        self.env("AXON_FD_IN", "4");
        self.env("AXON_FD_OUT", "5");

        self.before_exec(move || {
            dup2(i2, 4.into())
                .map_err(|e| Error::new(ErrorKind::Other, e))
                ?;
            dup2(o2, 5.into())
                .map_err(|e| Error::new(ErrorKind::Other, e))
                ?;
            Ok(())
        });

        let child = self.spawn()?;

        axon_in.shutdown(Shutdown::Write)?;
        axon_out.shutdown(Shutdown::Read)?;

        let io = AxiomIo {
            axon_in,
            axon_out,
        };


        Ok((child, io))
    }

}


pub struct AxiomIo {
    axon_in: UnixDatagram,
    axon_out: UnixDatagram,
}

impl AxiomIo {
    pub fn make_async(&mut self) -> io::Result<()> {
        use nix::fcntl::{fcntl, FdFlag, OFlag};
        use nix::fcntl::FcntlArg::{F_SETFD, F_SETFL};

        let infd = self.axon_in.as_raw_fd();
        fcntl(infd, F_SETFD(FdFlag::FD_CLOEXEC))
            .map_err(|e| Error::new(ErrorKind::Other, e))
            ?;
        fcntl(infd, F_SETFL(OFlag::O_NONBLOCK))
            .map_err(|e| Error::new(ErrorKind::Other, e))
            ?;

        Ok(())
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        match how {
            Shutdown::Read => {
                self.axon_in.shutdown(Shutdown::Both)?;
            },
            Shutdown::Write => {
                self.axon_out.shutdown(Shutdown::Both)?;
            },
            Shutdown::Both => {
                self.axon_in.shutdown(Shutdown::Both)?;
                self.axon_out.shutdown(Shutdown::Both)?;
            },
        }
        Ok(())
    }
}

impl Read for AxiomIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.axon_in.recv(buf)
    }
}

impl Write for AxiomIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.axon_out.send(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        fsync(self.axon_out.as_raw_fd())
            .map_err(|e| Error::new(ErrorKind::Other, e))
    }
}


impl AsyncRead  for AxiomIo {}
impl AsyncWrite for AxiomIo {
    fn shutdown(&mut self) -> Result<Async<()>, tokio::io::Error> {
        AxiomIo::shutdown(self, Shutdown::Both)?;
        Ok(Async::Ready(()))
    }
}

pub fn child() -> Result<AxiomIo, Error> {
    let axon_fd_out : RawFd = env::var("AXON_FD_OUT")
        .map_err(|_| Error::new(ErrorKind::Other, format!("AXON_FD_OUT missing. executable needs to be spawned from an axon host")))?
        .parse()
        .map_err(|e| Error::new(ErrorKind::Other, e))?;

    let axon_fd_in : RawFd = env::var("AXON_FD_IN")
        .map_err(|_| Error::new(ErrorKind::Other, format!("AXON_FD_IN  missing. executable needs to be spawned from an axon host")))?
        .parse()
        .map_err(|e| Error::new(ErrorKind::Other, e))?;

    let axon_in  = unsafe{UnixDatagram::from_raw_fd(axon_fd_in)};
    let axon_out = unsafe{UnixDatagram::from_raw_fd(axon_fd_out)};
    axon_in.shutdown(Shutdown::Write)?;
    axon_out.shutdown(Shutdown::Read)?;

    Ok(AxiomIo {
        axon_in,
        axon_out,
    })
}


