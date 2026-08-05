use std::sync::atomic::{AtomicBool, Ordering};

use wroid_daemon::ipc::DaemonServer;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn request_stop(_signal: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_signal_handlers()?;
    let mut server = DaemonServer::bind_default()?;
    println!(
        "wroidd protocol {} listening for the current user",
        wroid_daemon::ipc::PROTOCOL_VERSION
    );
    server.serve_until(&STOP)?;
    Ok(())
}

fn install_signal_handlers() -> Result<(), std::io::Error> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: zero is a valid initial representation for sigaction before
        // its mask, flags, and handler are initialized below.
        let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = request_stop as *const () as libc::sighandler_t;
        action.sa_flags = 0;
        // SAFETY: action.sa_mask is a valid writable signal set.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: action is fully initialized and the handler remains valid
        // for the lifetime of the process.
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
