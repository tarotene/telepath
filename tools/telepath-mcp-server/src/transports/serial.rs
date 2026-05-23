use anyhow::Context;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
    read_deadline: Option<Instant>,
}

impl SerialTransport {
    /// Open the serial port at `path` with the given baud rate.
    pub fn open(path: &Path, baud: u32) -> anyhow::Result<Self> {
        let port = serialport::new(path.to_string_lossy(), baud)
            .timeout(Duration::from_millis(100))
            .open()
            .with_context(|| format!("failed to open serial port '{}'", path.display()))?;
        Ok(Self {
            port,
            read_deadline: None,
        })
    }

    pub fn set_read_deadline(&mut self, timeout: Duration) {
        self.read_deadline = Some(Instant::now() + timeout);
    }
}

impl Read for SerialTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // serialport::open uses a 100ms blocking read timeout; retry until a byte
        // arrives or the deadline elapses, mirroring the RTT transport's pattern.
        loop {
            if let Some(deadline) = self.read_deadline {
                if Instant::now() > deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "serial read deadline exceeded",
                    ));
                }
            }
            match self.port.read(buf) {
                Ok(n) if n > 0 => return Ok(n),
                Ok(_) => {}
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    // serialport timeout — no data yet, retry
                }
                Err(e) => return Err(e),
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Write for SerialTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.port.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }
}
