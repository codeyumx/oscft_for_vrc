//! Child process plumbing: Windows Job Object (kill-on-close), line-oriented output capture.

use std::io::{self, BufRead, BufReader, Read};
use std::process::Child;

/// Puts `child` into the process-wide kill-on-close job so it dies with the host, even if the host is killed.
#[cfg(windows)]
pub(crate) fn assign_to_job(child: &Child) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use std::sync::LazyLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    struct Job(HANDLE);
    // SAFETY: a job handle is just a kernel object reference; it is valid on every thread and never closed.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: LazyLock<Result<Job, i32>> = LazyLock::new(|| {
        // SAFETY: plain Win32 calls with valid arguments; `info` outlives the call and its size is passed.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
            }
            Ok(Job(handle))
        }
    });
    let job = &*JOB;
    let job = job.as_ref().map_err(|code| io::Error::from_raw_os_error(*code))?;
    // SAFETY: both handles are valid; the child handle stays open while `child` is alive.
    if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Children also exit on their own when the connection closes or the parent pid disappears (`--parent-pid`).
#[cfg(not(windows))]
pub(crate) fn assign_to_job(_child: &Child) -> io::Result<()> {
    Ok(())
}

/// Calls `on_line` for each line (lossy UTF-8, trailing CR/LF trimmed, blanks skipped) until EOF.
pub(crate) fn pump_lines(stream: impl Read, mut on_line: impl FnMut(&str)) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\r', '\n']);
                if !line.trim().is_empty() {
                    on_line(line);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pump_lines_splits_trims_and_skips_blank_lines() {
        let mut lines = Vec::new();
        pump_lines(&b"one\r\n\r\ntwo\nlast without newline"[..], |l| lines.push(l.to_owned()));
        assert_eq!(lines, ["one", "two", "last without newline"]);
    }
}
