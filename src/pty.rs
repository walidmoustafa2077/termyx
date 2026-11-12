//! Shell session creation and byte transport.

use std::io::{Read, Write};
use std::path::Path;
use std::thread;

use anyhow::Result;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use winit::event_loop::EventLoopProxy;

use crate::config::ShellConfig;
use crate::event::UserEvent;

/// A shell running in a pseudo-terminal.
///
/// One reader thread forwards output bytes verbatim and one waiter thread
/// reports the shell exiting; the UI thread owns this value and never touches
/// the PTY reader directly.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    /// Kills the shell when the session is dropped. The waiter thread owns the
    /// child and reaps it.
    killer: Box<dyn ChildKiller + Send + Sync>,
    _reader: thread::JoinHandle<()>,
    _waiter: thread::JoinHandle<()>,
}

impl PtySession {
    /// Spawns the configured shell and forwards its output, tagged with
    /// `session`, through `proxy`.
    ///
    /// `directory` is the shell's working directory, or `None` for the user's
    /// home. `autocorrect` is exported to the child as
    /// `TERMYX_AUTOCORRECT`, so the PowerShell module agrees with the
    /// window's setting.
    pub fn spawn(
        config: &ShellConfig,
        directory: Option<&Path>,
        autocorrect: bool,
        cols: u16,
        rows: u16,
        proxy: EventLoopProxy<UserEvent>,
        session: u64,
    ) -> Result<Self> {
        let exit_proxy = proxy.clone();
        Self::spawn_with(
            config,
            directory,
            autocorrect,
            cols,
            rows,
            move |bytes| {
                let _ = proxy.send_event(UserEvent::PtyOutput { session, bytes });
            },
            move || {
                let _ = exit_proxy.send_event(UserEvent::Exited { session });
            },
        )
    }

    /// Spawns the configured shell and forwards output chunks to `on_output`,
    /// then calls `on_exit` once the reader loop ends.
    ///
    /// `autocorrect` enables the PowerShell module's on-enter correction pass
    /// for the child it starts. `directory` is the shell's working directory;
    /// `None` leaves the PTY crate's default (the user's home) in place.
    ///
    /// The reader thread owns both callbacks and exits when the PTY closes.
    pub fn spawn_with<F, G>(
        config: &ShellConfig,
        directory: Option<&Path>,
        autocorrect: bool,
        cols: u16,
        rows: u16,
        on_output: F,
        on_exit: G,
    ) -> Result<Self>
    where
        F: Fn(Vec<u8>) + Send + 'static,
        G: FnOnce() + Send + 'static,
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
        // The PTY crate starts the child in the user's home directory unless a
        // directory is set explicitly, so process-wide `chdir` is not enough.
        if let Some(directory) = directory {
            command.cwd(directory);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "termyx");
        // The PowerShell module keys its prompt marks and autocorrect on these;
        // exporting them here is what makes the child agree with the setting
        // the window is using now.
        command.env("TERMYX", "1");
        command.env("TERMYX_AUTOCORRECT", if autocorrect { "1" } else { "0" });

        let mut child = pair.slave.spawn_command(command)?;
        let killer = child.clone_killer();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => on_output(buffer[..n].to_vec()),
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => {
                        tracing::warn!(%err, "pty reader ended");
                        break;
                    }
                }
            }
        });
        // A Windows pseudoconsole keeps its output pipe open after the child
        // exits, so the reader never sees EOF there. Waiting on the child is the
        // one exit signal both platforms give, and it reaps the child so `Drop`
        // only has to kill.
        let waiter_thread = thread::spawn(move || {
            let _ = child.wait();
            on_exit();
        });

        Ok(Self {
            master: pair.master,
            writer,
            killer,
            _reader: reader_thread,
            _waiter: waiter_thread,
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
        // The waiter thread owns the child and reaps it; dropping the session
        // only has to stop the shell.
        let _ = self.killer.kill();
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
    fn the_child_starts_in_the_requested_directory() {
        let directory = std::env::temp_dir().join(format!("termyx-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create the test directory");
        let mut shell = ShellConfig::default();
        #[cfg(windows)]
        {
            shell.program = "cmd.exe".into();
            shell.args = vec!["/C".into(), "cd".into()];
        }
        #[cfg(not(windows))]
        {
            shell.program = "/bin/sh".into();
            shell.args = vec!["-c".into(), "pwd".into()];
        }

        let (sender, receiver) = mpsc::channel();
        let _session = PtySession::spawn_with(
            &shell,
            Some(directory.as_path()),
            false,
            80,
            24,
            move |bytes| {
                let _ = sender.send(bytes);
            },
            || {},
        )
        .expect("spawn pty");

        let expected = directory.display().to_string();
        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    output.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&output).contains(&expected) {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let matched = String::from_utf8_lossy(&output).contains(&expected);
        let _ = std::fs::remove_dir_all(&directory);
        assert!(
            matched,
            "pty output was {:?}, expected the working directory {:?}",
            String::from_utf8_lossy(&output),
            expected
        );
    }

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
        let _session = PtySession::spawn_with(
            &shell,
            None,
            false,
            80,
            24,
            move |bytes| {
                let _ = sender.send(bytes);
            },
            || {},
        )
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

    #[test]
    fn the_exit_callback_runs_when_the_shell_ends() {
        let mut shell = ShellConfig::default();
        #[cfg(windows)]
        {
            shell.program = "cmd.exe".into();
            shell.args = vec!["/C".into(), "exit".into()];
        }
        #[cfg(not(windows))]
        {
            shell.program = "/bin/sh".into();
            shell.args = vec!["-c".into(), "exit".into()];
        }

        let (sender, receiver) = mpsc::channel();
        let _session = PtySession::spawn_with(
            &shell,
            None,
            false,
            80,
            24,
            |_| {},
            move || {
                let _ = sender.send(());
            },
        )
        .expect("spawn pty");

        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the reader must report the shell exit");
    }
}
