//! One line of text with open file descriptors attached, over a unix stream socket (SCM_RIGHTS).
//! This is how `shabnam-send` gives the daemon the files to send: the requester opens them with
//! its own rights, the daemon only ever reads what it was handed and never looks a name up.
//!
//! The two sides take turns (one message, one answer), so a message and its descriptors always
//! arrive together and alone: bytes after the line's end, or more descriptors than asked for,
//! are a protocol error and the caller drops the connection.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

/// Most descriptors taken from one message; more is an error.
pub const MAX_FDS: usize = 4;

fn space(fds: usize) -> usize {
    unsafe { libc::CMSG_SPACE((fds * std::mem::size_of::<RawFd>()) as u32) as usize }
}

/// Sends `line` (which ends in a newline) with `fd` attached, if any.
pub fn send_line(stream: &UnixStream, line: &str, fd: Option<RawFd>) -> io::Result<()> {
    let bytes = line.as_bytes();
    let mut iov = libc::iovec { iov_base: bytes.as_ptr() as *mut libc::c_void, iov_len: bytes.len() };
    let mut control = vec![0u8; space(1)];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    if let Some(fd) = fd {
        message.msg_control = control.as_mut_ptr() as *mut libc::c_void;
        message.msg_controllen = control.len() as _;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::copy_nonoverlapping(&fd as *const RawFd as *const u8, libc::CMSG_DATA(header), std::mem::size_of::<RawFd>());
        }
    }
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, flags) };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    // The descriptor went with the first byte; the rest of a long line is plain data.
    if (sent as usize) < bytes.len() {
        use std::io::Write;
        (&*stream).write_all(&bytes[sent as usize..])?;
    }
    Ok(())
}

/// Receives one line (without its newline, at most `most` bytes) and the descriptors that came
/// with it. `Ok(None)`: the peer closed the connection between messages.
pub fn recv_line(stream: &UnixStream, most: usize) -> io::Result<Option<(String, Vec<OwnedFd>)>> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut line = Vec::new();
    let mut fds: Vec<OwnedFd> = Vec::new();
    loop {
        let mut chunk = vec![0u8; most + 1 - line.len().min(most)];
        let mut iov = libc::iovec { iov_base: chunk.as_mut_ptr() as *mut libc::c_void, iov_len: chunk.len() };
        let mut control = vec![0u8; space(MAX_FDS)];
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr() as *mut libc::c_void;
        message.msg_controllen = control.len() as _;
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_CMSG_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = 0;
        let got = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, flags) };
        if got < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // Whatever came attached is taken over first, so that it is closed on every way out.
        unsafe {
            let mut header = libc::CMSG_FIRSTHDR(&message);
            while !header.is_null() {
                if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                    let bytes = (*header).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                    for i in 0..bytes / std::mem::size_of::<RawFd>() {
                        let mut fd: RawFd = -1;
                        std::ptr::copy_nonoverlapping(libc::CMSG_DATA(header).add(i * std::mem::size_of::<RawFd>()), &mut fd as *mut RawFd as *mut u8, std::mem::size_of::<RawFd>());
                        if fd >= 0 {
                            #[cfg(not(target_os = "linux"))]
                            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                            fds.push(OwnedFd::from_raw_fd(fd));
                        }
                    }
                }
                header = libc::CMSG_NXTHDR(&message, header);
            }
        }
        if message.msg_flags & libc::MSG_CTRUNC != 0 || fds.len() > MAX_FDS {
            return Err(bad("too many descriptors"));
        }
        if got == 0 {
            return if line.is_empty() && fds.is_empty() { Ok(None) } else { Err(bad("the connection ended inside a message")) };
        }
        line.extend_from_slice(&chunk[..got as usize]);
        if let Some(end) = line.iter().position(|b| *b == b'\n') {
            if end + 1 != line.len() {
                return Err(bad("more than one message at a time"));
            }
            line.truncate(end);
            return Ok(Some((String::from_utf8_lossy(&line).into_owned(), fds)));
        }
        if line.len() > most {
            return Err(bad("the line is too long"));
        }
    }
}

/// What kind of thing a received descriptor is: `Some(size)` for a regular file open for
/// reading, None for anything else (a directory, a pipe, a device, a socket, write-only).
pub fn readable_regular_file(fd: &OwnedFd) -> Option<u64> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0 || stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return None;
    }
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    let mode = flags & libc::O_ACCMODE;
    #[cfg(target_os = "linux")]
    let path_only = flags & libc::O_PATH != 0;
    #[cfg(not(target_os = "linux"))]
    let path_only = false;
    (flags >= 0 && !path_only && (mode == libc::O_RDONLY || mode == libc::O_RDWR)).then_some(stat.st_size.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, Write};

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("shabnam-fdpass-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_file_travels_with_its_line() {
        let (a, b) = UnixStream::pair().unwrap();
        let path = temp("a");
        std::fs::write(&path, "hello over there").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        send_line(&a, "{\"t\":\"file\",\"name\":\"a\"}\n", Some(file.as_raw_fd())).unwrap();
        let (line, mut fds) = recv_line(&b, 8192).unwrap().unwrap();
        assert_eq!(line, "{\"t\":\"file\",\"name\":\"a\"}");
        assert_eq!(fds.len(), 1);
        let fd = fds.pop().unwrap();
        assert_eq!(readable_regular_file(&fd), Some(16));
        // The file is gone by name; what was handed over is still the file.
        std::fs::remove_file(&path).unwrap();
        let mut text = String::new();
        std::fs::File::from(fd).read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello over there");
        // A line without a descriptor, then the end.
        send_line(&a, "{\"t\":\"go\"}\n", None).unwrap();
        let (line, fds) = recv_line(&b, 8192).unwrap().unwrap();
        assert!(line == "{\"t\":\"go\"}" && fds.is_empty());
        drop(a);
        assert!(recv_line(&b, 8192).unwrap().is_none());
    }

    #[test]
    fn out_of_turn_or_overlong_is_an_error() {
        let (a, b) = UnixStream::pair().unwrap();
        (&a).write_all(b"one\ntwo\n").unwrap();
        assert!(recv_line(&b, 8192).is_err(), "two messages at once");
        let (a, b) = UnixStream::pair().unwrap();
        (&a).write_all(&vec![b'x'; 300]).unwrap();
        assert!(recv_line(&b, 100).is_err(), "longer than allowed");
        let (a, b) = UnixStream::pair().unwrap();
        (&a).write_all(b"half").unwrap();
        drop(a);
        assert!(recv_line(&b, 100).is_err(), "ended inside a message");
    }

    #[test]
    fn only_readable_regular_files_count() {
        let path = temp("b");
        std::fs::write(&path, "12345").unwrap();
        let read = OwnedFd::from(std::fs::File::open(&path).unwrap());
        assert_eq!(readable_regular_file(&read), Some(5));
        let write_only = OwnedFd::from(std::fs::OpenOptions::new().write(true).open(&path).unwrap());
        assert_eq!(readable_regular_file(&write_only), None);
        let dir = OwnedFd::from(std::fs::File::open(std::env::temp_dir()).unwrap());
        assert_eq!(readable_regular_file(&dir), None);
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(readable_regular_file(&OwnedFd::from(a)), None);
        let mut file = std::fs::File::open(&path).unwrap();
        file.seek(std::io::SeekFrom::End(0)).unwrap();
        assert_eq!(readable_regular_file(&OwnedFd::from(file)), Some(5), "the size is the file's, wherever its offset is");
        std::fs::remove_file(&path).unwrap();
    }
}
