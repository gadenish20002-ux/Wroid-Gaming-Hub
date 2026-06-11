use std::io::{self, Write};

use anyhow::{Context, Result};

pub(crate) fn write_stdout(output: &str) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    write_output(&mut handle, output)
}

pub(crate) fn write_output(writer: &mut impl Write, output: &str) -> Result<()> {
    match writer.write_all(output.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error).context("failed to write command output"),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use super::*;

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_output_treats_broken_pipe_as_success() {
        let mut writer = BrokenPipeWriter;

        write_output(&mut writer, "com.example.game\n").unwrap();
    }
}
