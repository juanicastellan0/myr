fn run_app(
    run_tui: impl FnOnce() -> Result<(), myr_tui::TuiError>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = myr_core::domain_name();
    let _ = myr_adapters::adapter_name();
    run_tui()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_app(myr_tui::run)
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::run_app;

    #[test]
    fn run_app_returns_ok_when_tui_runner_succeeds() {
        let result = run_app(|| Ok(()));
        assert!(result.is_ok());
    }

    #[test]
    fn run_app_propagates_tui_errors() {
        let result = run_app(|| Err(myr_tui::TuiError::Io(io::Error::other("boom"))));
        assert!(result.is_err());
    }
}
