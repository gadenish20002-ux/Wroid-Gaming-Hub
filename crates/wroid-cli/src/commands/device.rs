use anyhow::Result;

use crate::backend::InputExecutor;
use crate::cli::InputBackend;
use crate::device::{detect_device_density, detect_device_screen, device_info_output};
use crate::output::write_stdout;

pub(crate) fn device_screen(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<()> {
    let resolution = detect_device_screen(input_executor, backend)?;
    println!("Screen: {resolution}");
    Ok(())
}

pub(crate) fn device_density(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<()> {
    let density = detect_device_density(input_executor, backend)?;
    println!("Density: {density}");
    Ok(())
}

pub(crate) fn device_info(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<()> {
    write_stdout(&device_info_output(input_executor, backend)?)
}
