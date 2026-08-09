//! Platform terminal operations.
//!
//! This module owns the OS terminal handle and its saved settings. It has no
//! subprocess path, so a failed helper process cannot leave the host terminal
//! in raw mode.

use std::io;

#[cfg(unix)]
mod unix {
    use super::io;
    use std::cell::UnsafeCell;
    use std::fs::File;
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    const SIGNALS: [libc::c_int; 12] = [
        libc::SIGINT,
        libc::SIGTERM,
        libc::SIGHUP,
        libc::SIGQUIT,
        libc::SIGABRT,
        libc::SIGILL,
        libc::SIGTRAP,
        libc::SIGBUS,
        libc::SIGFPE,
        libc::SIGSEGV,
        libc::SIGPIPE,
        libc::SIGSYS,
    ];

    static SIGNAL_ACTIVE: AtomicBool = AtomicBool::new(false);
    static SIGNAL_READY: AtomicBool = AtomicBool::new(false);
    static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

    struct SignalTermios(UnsafeCell<MaybeUninit<libc::termios>>);

    // The value is written before SIGNAL_READY is published and then only
    // read by the signal handler until the guard is disarmed.
    unsafe impl Sync for SignalTermios {}

    static SIGNAL_TERMIOS: SignalTermios =
        SignalTermios(UnsafeCell::new(MaybeUninit::uninit()));

    pub(crate) struct TerminalMode {
        tty: File,
        saved: libc::termios,
        signals: SignalGuard,
    }

    impl TerminalMode {
        pub(crate) fn enter() -> io::Result<Self> {
            let tty = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")?;
            let fd = tty.as_raw_fd();
            let saved = read_settings(fd)?;
            let signals = SignalGuard::install(fd, saved)?;

            let mut raw = saved;
            unsafe {
                libc::cfmakeraw(&mut raw);
            }
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
                drop(signals);
                return Err(io::Error::last_os_error())
            }

            Ok(Self { tty, saved, signals })
        }

        pub(crate) fn restore(&mut self) -> io::Result<()> {
            let result = set_settings(self.tty.as_raw_fd(), &self.saved);
            self.signals.disarm();
            result
        }
    }

    impl Drop for TerminalMode {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }

    pub(super) fn size() -> Option<(u16, u16)> {
        let tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .ok()?;
        let mut window = MaybeUninit::<libc::winsize>::zeroed();
        let result = unsafe {
            libc::ioctl(
                tty.as_raw_fd(),
                libc::TIOCGWINSZ,
                window.as_mut_ptr(),
            )
        };
        if result != 0 {
            return None
        }
        let window = unsafe { window.assume_init() };
        if window.ws_row == 0 || window.ws_col == 0 {
            return None
        }
        Some((window.ws_row, window.ws_col))
    }

    fn read_settings(fd: libc::c_int) -> io::Result<libc::termios> {
        let mut settings = MaybeUninit::uninit();
        if unsafe { libc::tcgetattr(fd, settings.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error())
        }
        Ok(unsafe { settings.assume_init() })
    }

    fn set_settings(fd: libc::c_int, settings: &libc::termios) -> io::Result<()> {
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, settings) } != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    struct SignalGuard {
        previous: [libc::sigaction; SIGNALS.len()],
        installed: usize,
    }

    impl SignalGuard {
        fn install(fd: libc::c_int, saved: libc::termios) -> io::Result<Self> {
            if SIGNAL_ACTIVE.swap(true, Ordering::AcqRel) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "another raw terminal session is active",
                ))
            }
            SIGNAL_FD.store(fd, Ordering::Release);
            unsafe {
                ptr::write(SIGNAL_TERMIOS.0.get(), MaybeUninit::new(saved));
            }

            let mut guard = Self {
                previous: unsafe { std::mem::zeroed() },
                installed: 0,
            };
            for (index, signal) in SIGNALS.iter().copied().enumerate() {
                let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
                action.sa_sigaction = signal_handler as *const () as usize;
                unsafe {
                    libc::sigemptyset(&mut action.sa_mask);
                }
                action.sa_flags = 0;
                if unsafe { libc::sigaction(signal, &action, &mut guard.previous[index]) } != 0 {
                    guard.restore_previous_handlers();
                    SIGNAL_READY.store(false, Ordering::Release);
                    SIGNAL_FD.store(-1, Ordering::Release);
                    SIGNAL_ACTIVE.store(false, Ordering::Release);
                    return Err(io::Error::last_os_error())
                }
                guard.installed += 1;
            }
            SIGNAL_READY.store(true, Ordering::Release);
            Ok(guard)
        }

        fn disarm(&mut self) {
            SIGNAL_READY.store(false, Ordering::Release);
            self.restore_previous_handlers();
            SIGNAL_FD.store(-1, Ordering::Release);
            SIGNAL_ACTIVE.store(false, Ordering::Release);
            self.installed = 0;
        }

        fn restore_previous_handlers(&self) {
            for (index, signal) in SIGNALS.iter().copied().enumerate().take(self.installed) {
                unsafe {
                    libc::sigaction(signal, &self.previous[index], ptr::null_mut());
                }
            }
        }
    }

    impl Drop for SignalGuard {
        fn drop(&mut self) {
            self.disarm();
        }
    }

    extern "C" fn signal_handler(signal: libc::c_int) -> ! {
        if SIGNAL_READY.load(Ordering::Acquire) {
            let fd = SIGNAL_FD.load(Ordering::Acquire);
            if fd >= 0 {
                unsafe {
                    libc::tcsetattr(
                        fd,
                        libc::TCSANOW,
                        (*SIGNAL_TERMIOS.0.get()).as_ptr(),
                    );
                }
            }
        }

        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
            libc::_exit(128 + signal);
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::io;
    use std::io::IsTerminal;
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut std::ffi::c_void;

    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
    const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ConsoleScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetConsoleMode(console_handle: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(console_handle: Handle, mode: u32) -> i32;
        fn GetConsoleScreenBufferInfo(
            console_handle: Handle,
            info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
    }

    pub(crate) struct TerminalMode {
        input_handle: Handle,
        input_saved: u32,
        output_handle: Handle,
        output_saved: u32,
    }

    impl TerminalMode {
        pub(crate) fn enter() -> io::Result<Self> {
            let input_handle = io::stdin().as_raw_handle();
            let output_handle = io::stdout().as_raw_handle();
            let mut input_saved = 0;
            let mut output_saved = 0;
            if unsafe { GetConsoleMode(input_handle, &mut input_saved) } == 0 {
                return Err(io::Error::last_os_error())
            }
            if unsafe { GetConsoleMode(output_handle, &mut output_saved) } == 0 {
                return Err(io::Error::last_os_error())
            }

            let mut raw_input = input_saved
                & !(ENABLE_PROCESSED_INPUT
                    | ENABLE_LINE_INPUT
                    | ENABLE_ECHO_INPUT
                    | ENABLE_QUICK_EDIT_MODE);
            raw_input |= ENABLE_EXTENDED_FLAGS | ENABLE_VIRTUAL_TERMINAL_INPUT;
            if unsafe { SetConsoleMode(input_handle, raw_input) } == 0 {
                return Err(io::Error::last_os_error())
            }

            let raw_output = output_saved | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if unsafe { SetConsoleMode(output_handle, raw_output) } == 0 {
                let error = io::Error::last_os_error();
                unsafe {
                    SetConsoleMode(input_handle, input_saved);
                }
                return Err(error)
            }

            Ok(Self {
                input_handle,
                input_saved,
                output_handle,
                output_saved,
            })
        }

        pub(crate) fn restore(&mut self) -> io::Result<()> {
            let output_result = if unsafe {
                SetConsoleMode(self.output_handle, self.output_saved)
            } == 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            };
            let input_result = if unsafe {
                SetConsoleMode(self.input_handle, self.input_saved)
            } == 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            };

            match (output_result, input_result) {
                (Err(error), _) | (Ok(()), Err(error)) => Err(error),
                (Ok(()), Ok(())) => Ok(()),
            }
        }
    }

    impl Drop for TerminalMode {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }

    pub(super) fn size() -> Option<(u16, u16)> {
        if !io::stdout().is_terminal() {
            return None
        }
        let handle = io::stdout().as_raw_handle();
        let mut info = ConsoleScreenBufferInfo {
            size: Coord { x: 0, y: 0 },
            cursor_position: Coord { x: 0, y: 0 },
            attributes: 0,
            window: SmallRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            maximum_window_size: Coord { x: 0, y: 0 },
        };
        if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
            return None
        }
        let columns = info.window.right - info.window.left + 1;
        let rows = info.window.bottom - info.window.top + 1;
        u16::try_from(rows).ok().zip(u16::try_from(columns).ok())
    }
}

#[cfg(not(any(unix, windows)))]
mod portable {
    use super::io;

    pub(crate) struct TerminalMode;

    impl TerminalMode {
        pub(crate) fn enter() -> io::Result<Self> {
            Ok(Self)
        }

        pub(crate) fn restore(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn size() -> Option<(u16, u16)> {
        None
    }
}

#[cfg(unix)]
pub(super) use unix::TerminalMode;
#[cfg(windows)]
pub(super) use windows::TerminalMode;
#[cfg(not(any(unix, windows)))]
pub(super) use portable::TerminalMode;

pub(super) fn size() -> Option<(u16, u16)> {
    #[cfg(unix)]
    {
        unix::size()
    }
    #[cfg(windows)]
    {
        windows::size()
    }
    #[cfg(not(any(unix, windows)))]
    {
        portable::size()
    }
}
