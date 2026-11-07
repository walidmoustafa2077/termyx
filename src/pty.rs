//! Shell session creation and byte transport.

use std::io::{Read, Write};
use std::thread;

use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use winit::event_loop::EventLoopProxy;

use crate::config::ShellConfig;
use crate::event::UserEvent;

/// A shell running in a pseudo-terminal.
///
/// One reader thread forwards output bytes verbatim; the UI thread owns this
/// value and never touches the PTY reader directly.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _reader: thread::JoinHandle<()>,
}

impl PtySession {
    /// Spawns the configured shell and forwards its output through `proxy`.
    pub fn spawn(
        config: &ShellConfig,
        cols: u16,
        rows: u16,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self> {
        Self::spawn_with(config, cols, rows, move |bytes| {
            let _ = proxy.send_event(UserEvent::PtyOutput(bytes));
        })
    }

    /// Spawns the configured shell and forwards output chunks to `on_output`.
    ///
    /// The reader thread owns the callback and exits when the PTY closes.
    pub fn spawn_with<F>(config: &ShellConfig, cols: u16, rows: u16, on_output: F) -> Result<Self>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let (program, args) = build_command(config);
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut command = CommandBuilder::new(program);
        for arg in args {
            command.arg(arg);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "termyx");

        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => on_output(buffer[..n].to_vec()),
                    Err(err) => {
                        tracing::debug!(%err, "pty reader ended");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            child,
            _reader: reader_thread,
        })
    }

    /// Writes keyboard input bytes to the shell in logical order.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Tells the shell that the visible terminal area changed size.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Resolves the program and arguments for the child shell.
fn build_command(config: &ShellConfig) -> (String, Vec<String>) {
    if !config.program.is_empty() {
        #[cfg(windows)]
        let is_cmd = config.program.to_ascii_lowercase().ends_with("cmd.exe");
        #[cfg(windows)]
        if is_cmd && !config.args.is_empty() {
            tracing::warn!(
                "cmd.exe started with explicit args; the UTF-8 code page is not injected, \
                 set it yourself (e.g. chcp 65001)"
            );
        }
        #[cfg(windows)]
        let args = if is_cmd && config.args.is_empty() {
            // Measured child output mode is 0x7: VT processing is enabled by
            // ConPTY, but DISABLE_NEWLINE_AUTO_RETURN (0x8) is not, and cannot
            // be set without a new Win32 dependency. Revisit in Task 9 if
            // edge-wrap artifacts appear. The remaining requirement is the
            // UTF-8 code page.
            vec!["/K".into(), "chcp 65001>nul".into()]
        } else {
            config.args.clone()
        };
        #[cfg(not(windows))]
        let args = config.args.clone();
        return (config.program.clone(), args);
    }

    #[cfg(windows)]
    {
        let program = if which("pwsh.exe") {
            "pwsh.exe"
        } else {
            "powershell.exe"
        };
        let args = vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; [Console]::InputEncoding=[Text.Encoding]::UTF8".into(),
        ];
        (program.into(), args)
    }

    #[cfg(not(windows))]
    {
        let program = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        (program, config.args.clone())
    }
}

#[cfg(windows)]
fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|path| path.join(program).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn spawns_a_command_and_reads_its_output() {
        let mut shell = ShellConfig::default();
        #[cfg(windows)]
        {
            shell.program = "cmd.exe".into();
            shell.args = vec!["/C".into(), "echo hello".into()];
        }
        #[cfg(not(windows))]
        {
            shell.program = "/bin/sh".into();
            shell.args = vec!["-c".into(), "echo hello".into()];
        }

        let (sender, receiver) = mpsc::channel();
        let _session = PtySession::spawn_with(&shell, 80, 24, move |bytes| {
            let _ = sender.send(bytes);
        })
        .expect("spawn pty");

        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    output.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&output).contains("hello") {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(
            String::from_utf8_lossy(&output).contains("hello"),
            "pty output was {:?}",
            String::from_utf8_lossy(&output)
        );
    }
}
